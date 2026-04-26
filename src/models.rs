use serde::{Deserialize, Serialize};

/// The side index of a puzzle piece (1=top, 2=right, 3=bottom, 4=left)
pub type SideIndex = u8;

pub fn side_name(index: SideIndex) -> &'static str {
    match index {
        1 => "Top",
        2 => "Right",
        3 => "Bottom",
        4 => "Left",
        _ => "Unknown",
    }
}

/// Classification of a puzzle piece side
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SideType {
    /// Flat edge (border of the puzzle)
    Linear,
    /// Concavity pointing inward (hole)
    ConcaveInward,
    /// Concavity pointing outward (tab)
    ConcaveOutward,
}

impl std::fmt::Display for SideType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SideType::Linear => write!(f, "Linear"),
            SideType::ConcaveInward => write!(f, "ConcaveInward (type 2)"),
            SideType::ConcaveOutward => write!(f, "ConcaveOutward (type 3)"),
        }
    }
}

/// A 2D point with floating point coordinates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn distance_to(&self, other: &Point) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

/// Metrics for a non-linear side (type 2 or 3)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcavityMetrics {
    /// The most extreme point of the concavity
    pub apex: Point,

    /// Euclidean distance from apex to corner_a (the "start" corner of this side)
    pub euclidean_dist_to_corner_a: f64,

    /// Euclidean distance from apex to corner_b (the "end" corner of this side)
    pub euclidean_dist_to_corner_b: f64,

    /// Distance along the perimeter from apex to corner_a
    pub perimeter_dist_to_corner_a: f64,

    /// Distance along the perimeter from apex to corner_b
    pub perimeter_dist_to_corner_b: f64,

    /// Perpendicular depth of the concavity from the baseline (line between the two corners)
    pub depth: f64,

    /// Total length of the side (perimeter between the two corners)
    pub side_perimeter_length: f64,

    /// Normalized position of the apex along the baseline [0.0 .. 1.0]
    /// 0.0 = at corner_a, 1.0 = at corner_b
    pub apex_position_ratio: f64,

    /// Area enclosed between the side contour and the baseline chord (pixels²)
    pub area: f64,
}

/// Description of one side of a puzzle piece
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PieceSide {
    /// Which side: 1=top, 2=right, 3=bottom, 4=left
    pub index: SideIndex,

    /// The type of this side
    pub side_type: SideType,

    /// The two corner points delimiting this side (in image coordinates)
    pub corner_a: Point,
    pub corner_b: Point,

    /// Metrics — only present if side_type is ConcaveInward or ConcaveOutward
    pub concavity: Option<ConcavityMetrics>,

    /// Contour points of this side (from corner_a to corner_b along the piece outline)
    #[serde(default)]
    pub contour: Vec<Point>,

    /// Sign (+1 or -1) indicating which side of the baseline is "inward" (toward centroid).
    /// Positive perp * centroid_side = inward. 0.0 means unknown (old descriptor).
    #[serde(default)]
    pub centroid_side: f64,
}

/// Full descriptor for one puzzle piece, stored as <name>.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PieceDescriptor {
    /// File stem, e.g. "000001"
    pub id: String,

    /// Original image filename, e.g. "000001.jpg"
    pub filename: String,

    /// The four sides, in order top/right/bottom/left
    pub sides: Vec<PieceSide>,

    /// Bounding box of the piece in the image (pixels)
    pub bbox_x: u32,
    pub bbox_y: u32,
    pub bbox_width: u32,
    pub bbox_height: u32,

    /// Centroid of the piece (pixels)
    pub centroid: Point,

    /// The four corner points of the piece (in image coordinates), clockwise from top-left
    pub corners: Vec<Point>,
}

/// A single match between two sides
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SideMatch {
    /// e.g. "000002-2"
    pub from_key: String,
    /// e.g. "000001-1"
    pub to_key: String,
    /// Compatibility score 0.0 .. 100.0
    pub score: f64,
    /// Clockwise rotation (0 / 90 / 180 / 270) to apply to the `to` piece
    /// so that its `to` side faces the `from` side.
    pub rotation: u16,
}

impl std::fmt::Display for SideMatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} -> {}|{:.1}%|rot{}°",
            self.from_key, self.to_key, self.score, self.rotation
        )
    }
}

/// The full output file: for each side key, all compatible matches above threshold
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OutputMatches {
    /// key: "pieceId-sideIndex", value: list of matches sorted by score desc
    pub matches: std::collections::BTreeMap<String, Vec<SideMatch>>,
}

impl OutputMatches {
    pub fn side_key(piece_id: &str, side_index: SideIndex) -> String {
        format!("{}-{}", piece_id, side_index)
    }
}

/// Weights for computing the compatibility score between two sides
#[derive(Debug, Clone)]
pub struct MatchWeights {
    /// Weight for euclidean distance similarity (0.0..1.0)
    pub euclidean_weight: f64,
    /// Weight for perimeter distance similarity (0.0..1.0)
    pub perimeter_weight: f64,
    /// Weight for depth similarity (0.0..1.0)
    pub depth_weight: f64,
    /// Weight for apex position ratio similarity (0.0..1.0)
    pub position_weight: f64,
    /// Weight for concavity area similarity (0.0..1.0)
    pub area_weight: f64,
    /// Weight for contour mean-diff score (Method 1)
    pub contour_mean_weight: f64,
    /// Weight for contour max-diff score (Method 2, Hausdorff)
    pub contour_max_weight: f64,
    /// Weight for baseline length (corner_a ↔ corner_b) similarity
    pub baseline_weight: f64,
    /// Relative threshold for contour gating (normalized by baseline_len)
    pub contour_threshold: f64,
}

impl Default for MatchWeights {
    fn default() -> Self {
        Self {
            euclidean_weight: 0.20,
            perimeter_weight: 0.20,
            depth_weight: 0.20,
            position_weight: 0.20,
            area_weight: 0.20,
            contour_mean_weight: 0.20,
            contour_max_weight: 0.20,
            baseline_weight: 0.10,
            contour_threshold: 0.15,
        }
    }
}

/// Coppie di pezzi confermate dall'utente, persistite in user.json.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserPairs {
    pub confirmed_pairs: Vec<(String, String)>,
}

impl UserPairs {
    /// Ritorna true se la coppia è già presente (in qualsiasi ordine).
    pub fn contains(&self, piece_a: &str, piece_b: &str) -> bool {
        self.confirmed_pairs.iter().any(|(a, b)| {
            (a == piece_a && b == piece_b) || (a == piece_b && b == piece_a)
        })
    }

    /// Aggiunge la coppia in forma canonica (ordinata) se non è già presente.
    pub fn add(&mut self, piece_a: &str, piece_b: &str) {
        if !self.contains(piece_a, piece_b) {
            let (a, b) = if piece_a <= piece_b {
                (piece_a.to_string(), piece_b.to_string())
            } else {
                (piece_b.to_string(), piece_a.to_string())
            };
            self.confirmed_pairs.push((a, b));
        }
    }
}
