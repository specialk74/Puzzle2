use anyhow::{bail, Context, Result};
use image::{DynamicImage, GrayImage, Rgb, RgbImage};
use log::{debug, info};
use opencv::{
    core::{self, Mat, Point as CvPoint, Point2f, Size, Vector},
    imgproc,
    prelude::*,
};

use crate::models::*;

/// Intermediate results from corner detection, used for debug images.
struct CornerDetectResult {
    corners: Vec<Point>,
    hull: Vec<Point>,
    quad: Vec<Point>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────────

/// Analyse a single puzzle piece image and return its full descriptor.
pub fn analyse_piece(
    _path: &std::path::Path,
    img: &DynamicImage,
    id: &str,
    filename: &str,
) -> Result<PieceDescriptor> {
    info!("[{}] Starting analysis", id);

    // 1. Convert to grayscale
    let gray = img.to_luma8();

    // 2. Gaussian blur (OpenCV, sigma=3.0 — matches previous imageproc behaviour)
    info!("[{}] Applying Gaussian blur (sigma=3.0)", id);
    let gray_mat = gray_to_mat(&gray)?;
    let mut blurred_mat = Mat::default();
    // gaussian_blur_def: sigma_y defaults to sigma_x, border to BORDER_REFLECT_101
    imgproc::gaussian_blur_def(&gray_mat, &mut blurred_mat, Size::new(0, 0), 3.0)?;

    // 3. Otsu binarisation — THRESH_BINARY_INV: piece pixels → 255, background → 0
    info!("[{}] Binarising image (Otsu)", id);
    let mut binary_mat = Mat::default();
    let otsu_val = imgproc::threshold(
        &blurred_mat,
        &mut binary_mat,
        0.0, // initial threshold (ignored with THRESH_OTSU)
        255.0,
        imgproc::THRESH_BINARY_INV | imgproc::THRESH_OTSU,
    )?;
    debug!("[{}] Otsu threshold = {:.0}", id, otsu_val);

    // 4. Find the largest contour (= piece outline)
    info!("[{}] Extracting contour", id);
    let contour =
        largest_contour_cv(&binary_mat).context("No contour found – is the image empty?")?;
    debug!("[{}] Contour has {} points", id, contour.len());

    // Debug image 4: contour drawn on original image (disabled)
    let _ = render_step4_contour(&img.to_rgb8(), &contour);

    // 5. Compute bounding box and centroid from the contour
    let (bbox_x, bbox_y, bbox_w, bbox_h) = bounding_box(&contour);
    let centroid = contour_centroid(&contour);
    info!(
        "[{}] BBox=({},{},{},{})  Centroid=({:.1},{:.1})",
        id, bbox_x, bbox_y, bbox_w, bbox_h, centroid.x, centroid.y
    );

    // Debug image 5: bounding box + centroid (disabled)
    let _ = render_step5_bbox_centroid(&img.to_rgb8(), bbox_x, bbox_y, bbox_w, bbox_h, &centroid);

    // 6. Detect the four corners of the piece
    info!("[{}] Detecting corners", id);
    let corner_result = find_corners(&centroid, &blurred_mat, &contour)
        .context("Could not detect 4 corners for this piece")?;
    let corners = corner_result.corners;
    for (i, c) in corners.iter().enumerate() {
        debug!("[{}] Corner[{}] = ({:.1}, {:.1})", id, i, c.x, c.y);
    }

    // Debug image 6: convex hull + simplified quad + snapped corners
    // let step6_img = render_step6_corners(
    //     &img.to_rgb8(),
    //     &corner_result.hull,
    //     &corner_result.quad,
    //     &corners,
    // );
    // io::save_debug_image(&step6_img, path, &format!("{}_6.jpg", id))?;

    // 7. Split contour into 4 sides using the detected corners
    info!("[{}] Splitting contour into 4 sides", id);
    let sides = build_sides(&contour, &corners, &centroid)?;
    for s in &sides {
        info!(
            "[{}] Side {} → {}{}",
            id,
            side_name(s.index),
            s.side_type,
            if let Some(m) = &s.concavity {
                format!(
                    "  apex=({:.1},{:.1}) depth={:.2} pos={:.2}",
                    m.apex.x, m.apex.y, m.depth, m.apex_position_ratio
                )
            } else {
                String::new()
            }
        );
    }

    // Debug image 7: 4 sides drawn in different colours
    // let step7_img = render_step7_sides(&img.to_rgb8(), &sides, &contour, &corners);
    // io::save_debug_image(&step7_img, path, &format!("{}_7.jpg", id))?;

    Ok(PieceDescriptor {
        id: id.to_string(),
        filename: filename.to_string(),
        sides,
        bbox_x,
        bbox_y,
        bbox_width: bbox_w,
        bbox_height: bbox_h,
        centroid,
        corners,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Image-type conversion
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a `GrayImage` (image crate) to a CV_8UC1 `Mat` (OpenCV).
///
/// `Mat::from_slice` builds a 1 × (w*h) row-vector; `reshape(1, h)` turns it
/// into the correct h × w matrix with the same data layout, and `try_clone`
/// gives us an owned copy independent of the slice lifetime.
fn gray_to_mat(img: &GrayImage) -> Result<Mat> {
    let (w, h) = img.dimensions();
    let data = img.as_raw();
    let _ = w; // used implicitly through the reshape
    let flat = Mat::from_slice(data.as_slice())?;
    let mat = flat.reshape(1, h as i32)?.try_clone()?;
    Ok(mat)
}

// ─────────────────────────────────────────────────────────────────────────────
// Contour extraction (OpenCV)
// ─────────────────────────────────────────────────────────────────────────────

fn largest_contour_cv(binary: &Mat) -> Result<Vec<Point>> {
    // findContours modifies the source image — clone first
    let mut src = binary.try_clone()?;

    let mut contours: Vector<Vector<CvPoint>> = Vector::new();
    imgproc::find_contours(
        &mut src,
        &mut contours,
        imgproc::RETR_EXTERNAL,
        imgproc::CHAIN_APPROX_NONE,
        CvPoint::new(0, 0),
    )?;

    debug!("Found {} raw contours", contours.len());

    let best = contours
        .iter()
        .max_by_key(|c| c.len())
        .ok_or_else(|| anyhow::anyhow!("No contours found"))?;

    let raw: Vec<Point> = best
        .iter()
        .map(|p| Point::new(p.x as f64, p.y as f64))
        .collect();

    // Simplify with OpenCV approxPolyDP (replaces the hand-rolled Douglas-Peucker)
    let decimated = approx_poly_dp_cv(&raw, 2.0)?;

    // Normalise to clockwise in image coordinates (Y-down)
    Ok(ensure_clockwise(decimated))
}

/// Contour simplification via OpenCV `approxPolyDP` (Douglas-Peucker).
fn approx_poly_dp_cv(pts: &[Point], epsilon: f64) -> Result<Vec<Point>> {
    if pts.is_empty() {
        return Ok(vec![]);
    }
    let cv_pts: Vector<Point2f> = pts
        .iter()
        .map(|p| Point2f::new(p.x as f32, p.y as f32))
        .collect();
    let mut approx: Vector<Point2f> = Vector::new();
    imgproc::approx_poly_dp(&cv_pts, &mut approx, epsilon, true)?;
    Ok(approx
        .iter()
        .map(|p| Point::new(p.x as f64, p.y as f64))
        .collect())
}

/// Convex hull via OpenCV `convexHull`.
fn convex_hull_cv(pts: &[Point]) -> Result<Vec<Point>> {
    if pts.len() < 3 {
        return Ok(pts.to_vec());
    }
    let cv_pts: Vector<Point2f> = pts
        .iter()
        .map(|p| Point2f::new(p.x as f32, p.y as f32))
        .collect();
    let mut hull: Vector<Point2f> = Vector::new();
    // clockwise=false → CCW hull (standard math orientation);
    // return_points=true → get actual Point2f values back.
    imgproc::convex_hull(&cv_pts, &mut hull, false, true)?;
    Ok(hull
        .iter()
        .map(|p| Point::new(p.x as f64, p.y as f64))
        .collect())
}

// ─────────────────────────────────────────────────────────────────────────────
// CW normalisation
// ─────────────────────────────────────────────────────────────────────────────

/// Ensure the contour is oriented clockwise in image coordinates (Y-down).
/// Shoelace area is positive for CW in image coords; reverse if negative.
fn ensure_clockwise(pts: Vec<Point>) -> Vec<Point> {
    let n = pts.len();
    if n < 3 {
        return pts;
    }
    let signed_area: f64 = (0..n)
        .map(|i| {
            let j = (i + 1) % n;
            pts[i].x * pts[j].y - pts[j].x * pts[i].y
        })
        .sum::<f64>()
        / 2.0;
    if signed_area < 0.0 {
        // CCW → reverse to make CW
        let mut rev = pts;
        rev.reverse();
        rev
    } else {
        pts
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Bounding box and centroid
// ─────────────────────────────────────────────────────────────────────────────

fn bounding_box(contour: &[Point]) -> (u32, u32, u32, u32) {
    let min_x = contour.iter().map(|p| p.x).fold(f64::MAX, f64::min);
    let max_x = contour.iter().map(|p| p.x).fold(f64::MIN, f64::max);
    let min_y = contour.iter().map(|p| p.y).fold(f64::MAX, f64::min);
    let max_y = contour.iter().map(|p| p.y).fold(f64::MIN, f64::max);
    (
        min_x as u32,
        min_y as u32,
        (max_x - min_x) as u32,
        (max_y - min_y) as u32,
    )
}

fn contour_centroid(contour: &[Point]) -> Point {
    let n = contour.len() as f64;
    let sx: f64 = contour.iter().map(|p| p.x).sum();
    let sy: f64 = contour.iter().map(|p| p.y).sum();
    Point::new(sx / n, sy / n)
}

pub fn find_corners(center: &Point, phase: &Mat, contour: &[Point]) -> Result<CornerDetectResult> {
    let max_corners = 4;
    let quality_level = 0.1;
    let use_harris_detector = true;
    let k = 0.1;
    let mut best_corners: Vector<Point2f> = Vector::new();
    let mut max_tot_distance = 0.0;
    let mut distance = 400.0; // 500.0;
    let center_f = Point2f::new(center.x as f32, center.y as f32);

    loop {
        let mut block_size = 30; // 40;
        loop {
            let mut corners: Vector<Point2f> = Vector::new();
            if let Err(err) = imgproc::good_features_to_track(
                phase,
                &mut corners,
                max_corners,
                quality_level,
                distance,
                &core::no_array(),
                block_size,
                use_harris_detector,
                k,
            ) {
                debug!("Error on find_corners (block_size {}): {}", block_size, err);
            }

            if corners.len() == 4 {
                let mut points: Vector<Point2f> = Vector::new();
                points.push(corners.get(0)? - center_f);
                points.push(corners.get(1)? - center_f);
                points.push(corners.get(2)? - center_f);
                points.push(corners.get(3)? - center_f);
                let tot_distance = core::norm_def(&points)?;

                if tot_distance > max_tot_distance {
                    best_corners = corners;
                    max_tot_distance = tot_distance;
                }
            }
            // else {
            //     debug!(
            //         "find_corners - Founded {} corners - {:?}",
            //         corners.len(),
            //         corners
            //     );
            // }

            block_size += 20;
            if block_size > 90 {
                break;
            }
        }
        distance += 20.0;
        if distance > 1000.0 {
            break;
        }
    }

    if best_corners.len() != 4 {
        bail!(
            "find_corners: could not detect exactly 4 corners but {}",
            best_corners.len()
        );
    }

    let mut corners = (0..4)
        .map(|i| {
            best_corners
                .get(i)
                .map(|p| Point::new(p.x as f64, p.y as f64))
        })
        .collect::<opencv::Result<Vec<Point>>>()?;

    // Sort into [TL, TR, BR, BL] order regardless of Harris detection order.
    // TL = min(x+y), BR = max(x+y), TR = min(y-x), BL = max(y-x)
    corners.sort_by(|a, b| (a.x + a.y).partial_cmp(&(b.x + b.y)).unwrap());
    let tl = corners[0].clone(); // min x+y
    let br = corners[3].clone(); // max x+y
    let mut mid = vec![corners[1].clone(), corners[2].clone()];
    mid.sort_by(|a, b| (a.y - a.x).partial_cmp(&(b.y - b.x)).unwrap());
    let tr = mid[0].clone(); // min y-x
    let bl = mid[1].clone(); // max y-x
    let corners = vec![tl, tr, br, bl];

    let hull = convex_hull_cv(contour)?;

    Ok(CornerDetectResult {
        corners,
        hull,
        quad: Vec::new(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Side building (pure Rust)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum Sides {
    Top,
    Right,
    Bottom,
    Left,
}

impl std::fmt::Display for Sides {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Sides::Top => write!(f, "Top"),
            Sides::Right => write!(f, "Right"),
            Sides::Bottom => write!(f, "Bottom"),
            Sides::Left => write!(f, "Left"),
        }
    }
}

impl From<Sides> for SideIndex {
    fn from(s: Sides) -> SideIndex {
        match s {
            Sides::Top => 1,
            Sides::Right => 2,
            Sides::Bottom => 3,
            Sides::Left => 4,
        }
    }
}

/// Split the contour into 4 sides and classify each one.
/// corners order: [TL(0), TR(1), BR(2), BL(3)]
fn build_sides(contour: &[Point], corners: &[Point], _centroid: &Point) -> Result<Vec<PieceSide>> {
    let n_c = corners.len() as f64;
    let body_centroid = Point::new(
        corners.iter().map(|c| c.x).sum::<f64>() / n_c,
        corners.iter().map(|c| c.y).sum::<f64>() / n_c,
    );

    let corner_indices: Vec<usize> = corners.iter().map(|c| closest_index(contour, c)).collect();
    debug!("Corner contour indices: {:?}", corner_indices);

    let n = contour.len();
    let side_defs: [(usize, usize, Sides); 4] = [
        (corner_indices[0], corner_indices[1], Sides::Top),
        (corner_indices[1], corner_indices[2], Sides::Right),
        (corner_indices[2], corner_indices[3], Sides::Bottom),
        (corner_indices[3], corner_indices[0], Sides::Left),
    ];

    let mut sides = Vec::new();
    for (start_idx, end_idx, side) in &side_defs {
        let segment = extract_segment(contour, *start_idx, *end_idx, n);
        debug!("[side {}] segment has {} points", side, segment.len());
        let corner_a = contour[*start_idx].clone();
        let corner_b = contour[*end_idx].clone();
        sides.push(classify_side(
            &segment,
            *side,
            corner_a,
            corner_b,
            &body_centroid,
        )?);
    }
    Ok(sides)
}

fn closest_index(contour: &[Point], target: &Point) -> usize {
    contour
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            a.distance_to(target)
                .partial_cmp(&b.distance_to(target))
                .unwrap()
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn extract_segment(contour: &[Point], start: usize, end: usize, n: usize) -> Vec<Point> {
    // The contour is CW-normalised, so going FORWARD (increasing index,
    // wrapping) from corner[start] to corner[end] always traces the correct
    // side — even when a large tab/socket makes that arc longer.
    let mut seg = Vec::new();
    let mut i = start;
    loop {
        seg.push(contour[i].clone());
        if i == end {
            break;
        }
        i = (i + 1) % n;
        if seg.len() > n + 1 {
            break; // safety guard against infinite loop
        }
    }
    seg
}

/// Classify a side segment as Linear, ConcaveInward or ConcaveOutward,
/// and compute all metrics.
fn classify_side(
    segment: &[Point],
    side_index: Sides,
    corner_a: Point,
    corner_b: Point,
    centroid: &Point,
) -> Result<PieceSide> {
    let contour = segment.to_vec();

    if segment.len() < 2 {
        return Ok(PieceSide {
            index: side_index.into(),
            side_type: SideType::Linear,
            corner_a,
            corner_b,
            concavity: None,
            contour,
            centroid_side: 0.0,
        });
    }

    let baseline_dx = corner_b.x - corner_a.x;
    let baseline_dy = corner_b.y - corner_a.y;
    let baseline_len = (baseline_dx * baseline_dx + baseline_dy * baseline_dy).sqrt();

    if baseline_len < 1e-6 {
        return Ok(PieceSide {
            index: side_index.into(),
            side_type: SideType::Linear,
            corner_a,
            corner_b,
            concavity: None,
            contour,
            centroid_side: 0.0,
        });
    }

    // Signed perpendicular distance of the centroid from the baseline line.
    // Tells us which side of the baseline is "inward" (toward piece centre).
    let centroid_num = (corner_b.y - corner_a.y) * centroid.x
        - (corner_b.x - corner_a.x) * centroid.y
        + corner_b.x * corner_a.y
        - corner_b.y * corner_a.x;
    let centroid_side = centroid_num.signum();

    // Per-point signed deviation from the baseline line.
    // Positive = inward (same side as centroid), negative = outward.
    let signed_devs: Vec<f64> = segment
        .iter()
        .map(|p| {
            let num = (corner_b.y - corner_a.y) * p.x - (corner_b.x - corner_a.x) * p.y
                + corner_b.x * corner_a.y
                - corner_b.y * corner_a.x;
            let perp = num / baseline_len;
            perp * centroid_side
        })
        .collect();

    let max_dev = signed_devs
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);
    let min_dev = signed_devs.iter().cloned().fold(f64::INFINITY, f64::min);

    // Linearity threshold: 5% of baseline length (relative, not fixed pixels).
    let linearity_threshold = baseline_len * 0.05;

    debug!(
        "[side {}] max_dev={:.2} min_dev={:.2}",
        side_index, max_dev, min_dev
    );

    if max_dev.abs() < linearity_threshold && min_dev.abs() < linearity_threshold {
        info!("[side {}] classified as Linear", side_index);
        return Ok(PieceSide {
            index: side_index.into(),
            side_type: SideType::Linear,
            corner_a,
            corner_b,
            concavity: None,
            contour,
            centroid_side,
        });
    }

    let (apex_idx, side_type) = if max_dev.abs() >= min_dev.abs() {
        // Dominant deviation is inward → ConcaveInward (socket)
        let idx = signed_devs
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0);
        (idx, SideType::ConcaveInward)
    } else {
        // Dominant deviation is outward → ConcaveOutward (tab)
        let idx = signed_devs
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0);
        (idx, SideType::ConcaveOutward)
    };

    let apex = segment[apex_idx].clone();
    let depth = max_dev.abs().max(min_dev.abs());

    let euclidean_dist_to_corner_a = apex.distance_to(&corner_a);
    let euclidean_dist_to_corner_b = apex.distance_to(&corner_b);

    let perimeter_dists = cumulative_perimeter(segment);
    let total_perimeter = *perimeter_dists.last().unwrap_or(&0.0);
    let apex_perimeter = perimeter_dists[apex_idx];
    let perimeter_dist_to_corner_a = apex_perimeter;
    let perimeter_dist_to_corner_b = total_perimeter - apex_perimeter;

    let proj_t = {
        let ax = apex.x - corner_a.x;
        let ay = apex.y - corner_a.y;
        let bx = corner_b.x - corner_a.x;
        let by = corner_b.y - corner_a.y;
        let t = (ax * bx + ay * by) / (bx * bx + by * by).max(1e-10);
        t.clamp(0.0, 1.0)
    };

    let area = concavity_area(segment);

    info!(
        "[side {}] classified as {}  apex=({:.1},{:.1}) depth={:.2} pos_ratio={:.2} area={:.1}",
        side_index, side_type, apex.x, apex.y, depth, proj_t, area
    );

    Ok(PieceSide {
        index: side_index.into(),
        side_type,
        corner_a,
        corner_b,
        concavity: Some(ConcavityMetrics {
            apex,
            euclidean_dist_to_corner_a,
            euclidean_dist_to_corner_b,
            perimeter_dist_to_corner_a,
            perimeter_dist_to_corner_b,
            depth,
            side_perimeter_length: total_perimeter,
            apex_position_ratio: proj_t,
            area,
        }),
        contour,
        centroid_side,
    })
}

/// Area enclosed between the side contour segment and the baseline chord,
/// computed with the Shoelace formula (closing the polygon via the baseline).
fn concavity_area(segment: &[Point]) -> f64 {
    let n = segment.len();
    if n < 3 {
        return 0.0;
    }
    // Shoelace: polygon is [segment[0], ..., segment[n-1]] closed back to segment[0]
    let signed: f64 = (0..n)
        .map(|i| {
            let j = (i + 1) % n;
            segment[i].x * segment[j].y - segment[j].x * segment[i].y
        })
        .sum::<f64>()
        / 2.0;
    signed.abs()
}

fn cumulative_perimeter(pts: &[Point]) -> Vec<f64> {
    let mut dists = vec![0.0f64];
    for i in 1..pts.len() {
        let d = pts[i].distance_to(&pts[i - 1]);
        dists.push(dists[i - 1] + d);
    }
    dists
}

// ─────────────────────────────────────────────────────────────────────────────
// Debug image rendering — pure Rust, using the image crate
// ─────────────────────────────────────────────────────────────────────────────

/// Step 4 debug: contour outline (green polyline + red sample dots).
fn render_step4_contour(original: &RgbImage, contour: &[Point]) -> RgbImage {
    let mut img = original.clone();
    let t = auto_thickness(&img);
    let n = contour.len();
    for i in 0..n {
        let a = &contour[i];
        let b = &contour[(i + 1) % n];
        draw_thick_line(
            &mut img,
            a.x as i32,
            a.y as i32,
            b.x as i32,
            b.y as i32,
            Rgb([0u8, 255u8, 0u8]),
            t,
        );
    }
    for p in contour {
        fill_circle(
            &mut img,
            p.x as i32,
            p.y as i32,
            (t * 2).max(3),
            Rgb([255u8, 0u8, 0u8]),
        );
    }
    img
}

/// Step 5 debug: bounding box (green) + centroid (red crosshair).
fn render_step5_bbox_centroid(
    original: &RgbImage,
    bbox_x: u32,
    bbox_y: u32,
    bbox_w: u32,
    bbox_h: u32,
    centroid: &Point,
) -> RgbImage {
    let mut img = original.clone();
    let t = auto_thickness(&img);
    draw_rect_thick(
        &mut img,
        bbox_x as i32,
        bbox_y as i32,
        bbox_w as i32,
        bbox_h as i32,
        Rgb([0u8, 220u8, 0u8]),
        t,
    );
    let r = t * 7;
    fill_circle(
        &mut img,
        centroid.x as i32,
        centroid.y as i32,
        r,
        Rgb([0u8, 0u8, 0u8]),
    );
    fill_circle(
        &mut img,
        centroid.x as i32,
        centroid.y as i32,
        r - t,
        Rgb([255u8, 40u8, 40u8]),
    );
    draw_thick_line(
        &mut img,
        centroid.x as i32 - r,
        centroid.y as i32,
        centroid.x as i32 + r,
        centroid.y as i32,
        Rgb([255u8, 255u8, 255u8]),
        (t / 2).max(1),
    );
    draw_thick_line(
        &mut img,
        centroid.x as i32,
        centroid.y as i32 - r,
        centroid.x as i32,
        centroid.y as i32 + r,
        Rgb([255u8, 255u8, 255u8]),
        (t / 2).max(1),
    );
    img
}

/// Step 6 debug: convex hull (cyan), simplified quad (yellow), snapped corners (blue).
fn render_step6_corners(
    original: &RgbImage,
    hull: &[Point],
    quad: &[Point],
    corners: &[Point],
) -> RgbImage {
    let mut img = original.clone();
    let t = auto_thickness(&img);

    // Convex hull — cyan thin polygon
    for i in 0..hull.len() {
        let a = &hull[i];
        let b = &hull[(i + 1) % hull.len()];
        draw_thick_line(
            &mut img,
            a.x as i32,
            a.y as i32,
            b.x as i32,
            b.y as i32,
            Rgb([0u8, 220u8, 220u8]),
            t,
        );
    }

    // Simplified quad — thick yellow polygon
    for i in 0..quad.len() {
        let a = &quad[i];
        let b = &quad[(i + 1) % quad.len()];
        draw_thick_line(
            &mut img,
            a.x as i32,
            a.y as i32,
            b.x as i32,
            b.y as i32,
            Rgb([255u8, 220u8, 0u8]),
            t * 2,
        );
    }

    // Snapped corners — blue dots with white outline
    let r = t * 6;
    for c in corners {
        fill_circle(
            &mut img,
            c.x as i32,
            c.y as i32,
            r,
            Rgb([255u8, 255u8, 255u8]),
        );
        fill_circle(
            &mut img,
            c.x as i32,
            c.y as i32,
            r - t,
            Rgb([30u8, 80u8, 255u8]),
        );
    }
    img
}

/// Step 7 debug: 4 sides in distinct colours, full contour in gray underneath.
fn render_step7_sides(
    original: &RgbImage,
    sides: &[PieceSide],
    contour: &[Point],
    corners: &[Point],
) -> RgbImage {
    let mut img = original.clone();
    let t = auto_thickness(&img);

    // Full contour in gray as background reference
    let n = contour.len();
    for i in 0..n {
        let a = &contour[i];
        let b = &contour[(i + 1) % n];
        draw_thick_line(
            &mut img,
            a.x as i32,
            a.y as i32,
            b.x as i32,
            b.y as i32,
            Rgb([120u8, 120u8, 120u8]),
            t,
        );
    }

    let side_colors: [Rgb<u8>; 4] = [
        Rgb([255u8, 50u8, 50u8]),  // Side 1 top    — red
        Rgb([50u8, 220u8, 50u8]),  // Side 2 right  — green
        Rgb([50u8, 100u8, 255u8]), // Side 3 bottom — blue
        Rgb([255u8, 160u8, 0u8]),  // Side 4 left   — orange
    ];

    for side in sides {
        let color = side_colors[(side.index as usize - 1) % 4];
        let r = t * 5;

        draw_thick_line(
            &mut img,
            side.corner_a.x as i32,
            side.corner_a.y as i32,
            side.corner_b.x as i32,
            side.corner_b.y as i32,
            color,
            t * 2,
        );

        // Corner A marker
        fill_circle(
            &mut img,
            side.corner_a.x as i32,
            side.corner_a.y as i32,
            r,
            Rgb([255u8, 255u8, 255u8]),
        );
        fill_circle(
            &mut img,
            side.corner_a.x as i32,
            side.corner_a.y as i32,
            r - t,
            color,
        );

        // Corner B marker
        fill_circle(
            &mut img,
            side.corner_b.x as i32,
            side.corner_b.y as i32,
            r,
            Rgb([255u8, 255u8, 255u8]),
        );
        fill_circle(
            &mut img,
            side.corner_b.x as i32,
            side.corner_b.y as i32,
            r - t,
            color,
        );

        // Midpoint dot
        let mx = ((side.corner_a.x + side.corner_b.x) / 2.0) as i32;
        let my = ((side.corner_a.y + side.corner_b.y) / 2.0) as i32;
        fill_circle(&mut img, mx, my, t * 2, color);
    }

    // Large white corner markers
    let r_big = t * 7;
    for c in corners {
        fill_circle(
            &mut img,
            c.x as i32,
            c.y as i32,
            r_big,
            Rgb([0u8, 0u8, 0u8]),
        );
        fill_circle(
            &mut img,
            c.x as i32,
            c.y as i32,
            r_big - t,
            Rgb([255u8, 255u8, 255u8]),
        );
    }
    img
}

/// Derive a sensible stroke thickness from the image size.
/// Rule: ~0.5% of the shorter dimension, minimum 3 px.
fn auto_thickness(img: &RgbImage) -> i32 {
    let (w, h) = img.dimensions();
    let shorter = w.min(h) as f64;
    ((shorter * 0.005).round() as i32).max(3)
}

/// Draw all extracted features onto a copy of the original image and return it.
pub fn render_debug_image(original: &DynamicImage, desc: &PieceDescriptor) -> RgbImage {
    let mut img = original.to_rgb8();
    let t = auto_thickness(&img);
    let r_corner = t * 6;
    let r_apex = t * 5;
    let r_mid = t * 3;
    let r_centroid = t * 7;

    // Bounding box (green)
    draw_rect_thick(
        &mut img,
        desc.bbox_x as i32,
        desc.bbox_y as i32,
        desc.bbox_width as i32,
        desc.bbox_height as i32,
        Rgb([0u8, 220u8, 0u8]),
        t,
    );

    // Side baselines — same colours as render_step7_sides
    // index: 1=Top red, 2=Right green, 3=Bottom blue, 4=Left orange
    let side_colors: [Rgb<u8>; 4] = [
        Rgb([255u8, 50u8, 50u8]),  // Top    — red
        Rgb([50u8, 220u8, 50u8]),  // Right  — green
        Rgb([50u8, 100u8, 255u8]), // Bottom — blue
        Rgb([255u8, 160u8, 0u8]),  // Left   — orange
    ];
    for side in &desc.sides {
        let color = side_colors[(side.index as usize - 1) % 4];
        draw_thick_line(
            &mut img,
            side.corner_a.x as i32,
            side.corner_a.y as i32,
            side.corner_b.x as i32,
            side.corner_b.y as i32,
            color,
            t,
        );

        if let Some(m) = &side.concavity {
            let mid_x = ((side.corner_a.x + side.corner_b.x) / 2.0) as i32;
            let mid_y = ((side.corner_a.y + side.corner_b.y) / 2.0) as i32;

            // Depth indicator (yellow)
            draw_thick_line(
                &mut img,
                mid_x,
                mid_y,
                m.apex.x as i32,
                m.apex.y as i32,
                Rgb([255u8, 255u8, 0u8]),
                t,
            );

            // Apex circle
            fill_circle(
                &mut img,
                m.apex.x as i32,
                m.apex.y as i32,
                r_apex,
                Rgb([0u8, 0u8, 0u8]),
            );
            fill_circle(
                &mut img,
                m.apex.x as i32,
                m.apex.y as i32,
                r_apex - t,
                Rgb([255u8, 230u8, 0u8]),
            );
        }

        // Side-midpoint dot (cyan)
        let mx = ((side.corner_a.x + side.corner_b.x) / 2.0) as i32;
        let my = ((side.corner_a.y + side.corner_b.y) / 2.0) as i32;
        fill_circle(&mut img, mx, my, r_mid, Rgb([0u8, 230u8, 230u8]));
    }

    // Corner markers (blue with white outline)
    for c in &desc.corners {
        fill_circle(
            &mut img,
            c.x as i32,
            c.y as i32,
            r_corner,
            Rgb([255u8, 255u8, 255u8]),
        );
        fill_circle(
            &mut img,
            c.x as i32,
            c.y as i32,
            r_corner - t,
            Rgb([30u8, 80u8, 255u8]),
        );
    }

    // Centroid (red crosshair inside a circle)
    fill_circle(
        &mut img,
        desc.centroid.x as i32,
        desc.centroid.y as i32,
        r_centroid,
        Rgb([0u8, 0u8, 0u8]),
    );
    fill_circle(
        &mut img,
        desc.centroid.x as i32,
        desc.centroid.y as i32,
        r_centroid - t,
        Rgb([255u8, 40u8, 40u8]),
    );
    draw_thick_line(
        &mut img,
        desc.centroid.x as i32 - r_centroid,
        desc.centroid.y as i32,
        desc.centroid.x as i32 + r_centroid,
        desc.centroid.y as i32,
        Rgb([255u8, 255u8, 255u8]),
        (t / 2).max(1),
    );
    draw_thick_line(
        &mut img,
        desc.centroid.x as i32,
        desc.centroid.y as i32 - r_centroid,
        desc.centroid.x as i32,
        desc.centroid.y as i32 + r_centroid,
        Rgb([255u8, 255u8, 255u8]),
        (t / 2).max(1),
    );

    img
}

// ─────────────────────────────────────────────────────────────────────────────
// Drawing primitives — thickness-aware, pure Rust
// ─────────────────────────────────────────────────────────────────────────────

fn fill_circle(img: &mut RgbImage, cx: i32, cy: i32, radius: i32, color: Rgb<u8>) {
    let (w, h) = img.dimensions();
    let r = radius.max(0);
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r * r {
                set_pixel(img, cx + dx, cy + dy, color, w, h);
            }
        }
    }
}

fn draw_thick_line(
    img: &mut RgbImage,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: Rgb<u8>,
    thickness: i32,
) {
    let steps = ((x1 - x0).abs().max((y1 - y0).abs()) as usize).max(1);
    let half = (thickness / 2).max(0);
    let (w, h) = img.dimensions();
    for i in 0..=steps {
        let t = i as f64 / steps as f64;
        let x = (x0 as f64 + t * (x1 - x0) as f64) as i32;
        let y = (y0 as f64 + t * (y1 - y0) as f64) as i32;
        for dy in -half..=half {
            for dx in -half..=half {
                set_pixel(img, x + dx, y + dy, color, w, h);
            }
        }
    }
}

fn draw_rect_thick(
    img: &mut RgbImage,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    color: Rgb<u8>,
    thickness: i32,
) {
    draw_thick_line(img, x, y, x + width, y, color, thickness);
    draw_thick_line(img, x + width, y, x + width, y + height, color, thickness);
    draw_thick_line(img, x + width, y + height, x, y + height, color, thickness);
    draw_thick_line(img, x, y + height, x, y, color, thickness);
}

fn set_pixel(img: &mut RgbImage, x: i32, y: i32, color: Rgb<u8>, w: u32, h: u32) {
    if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h {
        img.put_pixel(x as u32, y as u32, color);
    }
}
