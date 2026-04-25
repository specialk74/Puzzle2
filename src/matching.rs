use log::{debug, info};

use crate::models::*;

// ─────────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────────

/// Given all piece descriptors, compute all pairwise side matches above `threshold`.
/// Returns a map: side_key → Vec<SideMatch> sorted by score descending.
///
/// Rules:
///   - ConcaveInward (type 2) only matches ConcaveOutward (type 3) and vice-versa
///   - Linear sides are skipped
///   - Two sides from the same piece cannot match each other
///   - If piece A side X already matched piece B side Y, piece A cannot match piece B
///     on any other side pair (passed in as `existing_constraints` from output.json)
pub fn compute_matches(
    pieces: &[PieceDescriptor],
    weights: &MatchWeights,
    threshold: f64,
) -> OutputMatches {
    let mut result = OutputMatches::default();

    // Collect all non-linear sides as (piece_id, side_index, side_ref)
    let candidates: Vec<(&str, SideIndex, &PieceSide)> = pieces
        .iter()
        .flat_map(|p| {
            p.sides.iter().filter_map(move |s| {
                if s.side_type != SideType::Linear {
                    Some((p.id.as_str(), s.index, s))
                } else {
                    None
                }
            })
        })
        .collect();

    info!(
        "Matching: {} non-linear sides across {} pieces",
        candidates.len(),
        pieces.len()
    );

    // Compare each unique pair (i, j) once; insert match in both directions.
    for i in 0..candidates.len() {
        for j in (i + 1)..candidates.len() {
            let (pid_a, sidx_a, side_a) = &candidates[i];
            let (pid_b, sidx_b, side_b) = &candidates[j];

            if pid_a == pid_b {
                continue;
            }
            if !types_compatible(&side_a.side_type, &side_b.side_type) {
                continue;
            }

            let score = compute_score(side_a, side_b, weights);
            debug!(
                "  {}-{} ↔ {}-{} : score={:.1}%",
                pid_a, sidx_a, pid_b, sidx_b, score
            );

            if score < threshold {
                continue;
            }

            let key_a = OutputMatches::side_key(pid_a, *sidx_a);
            let key_b = OutputMatches::side_key(pid_b, *sidx_b);

            result.matches.entry(key_a.clone()).or_default().push(SideMatch {
                from_key: key_a.clone(),
                to_key: key_b.clone(),
                score,
                rotation: rotation_for_sides(*sidx_a, *sidx_b),
            });
            result.matches.entry(key_b.clone()).or_default().push(SideMatch {
                from_key: key_b.clone(),
                to_key: key_a.clone(),
                score,
                rotation: rotation_for_sides(*sidx_b, *sidx_a),
            });
        }
    }

    // Sort each entry by score descending and log
    for (pid, sidx, side) in &candidates {
        let key = OutputMatches::side_key(pid, *sidx);
        if let Some(matches) = result.matches.get_mut(&key) {
            matches.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
            info!(
                "[{}] side {} ({}) — {} matches above {:.0}%",
                pid, side_name(*sidx), side.side_type, matches.len(), threshold
            );
            for m in matches.iter() {
                info!("    {}", m);
            }
        } else {
            info!(
                "[{}] side {} ({}) — no matches above {:.0}%",
                pid, side_name(*sidx), side.side_type, threshold
            );
        }
    }

    // Apply the cross-piece exclusion rule:
    // If A-sideX is matched with B-sideY, no other side of A can match any side of B.
    apply_exclusion_rule(&mut result);

    result
}

/// Merge new matches into existing ones, keeping the highest scores.
/// Also removes entries that are now superseded by exclusion rules.
pub fn merge_matches(existing: &mut OutputMatches, new: OutputMatches) {
    for (key, new_matches) in new.matches {
        let entry = existing.matches.entry(key).or_default();
        for m in new_matches {
            // Add if not already present
            if !entry.iter().any(|e| e.to_key == m.to_key) {
                entry.push(m);
            }
        }
        entry.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    }
    apply_exclusion_rule(existing);
}

// ─────────────────────────────────────────────────────────────────────────────
// Contour profile comparison
// ─────────────────────────────────────────────────────────────────────────────

const PROFILE_N: usize = 100;

/// Project the side's contour onto its baseline, returning N resampled signed
/// perpendicular distances (normalized by baseline_len).
/// Positive = inward (toward centroid), negative = outward.
fn signed_profile(side: &PieceSide, n: usize) -> Vec<f64> {
    let contour = &side.contour;
    if contour.len() < 2 || side.centroid_side == 0.0 {
        return vec![0.0; n];
    }

    let ax = side.corner_a.x;
    let ay = side.corner_a.y;
    let bx = side.corner_b.x;
    let by = side.corner_b.y;
    let dx = bx - ax;
    let dy = by - ay;
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-12 {
        return vec![0.0; n];
    }
    let cs = side.centroid_side;

    let mut pts: Vec<(f64, f64)> = contour
        .iter()
        .map(|p| {
            let px = p.x - ax;
            let py = p.y - ay;
            let t = (px * dx + py * dy) / len2;
            // same perpendicular formula as classify_side, normalized by len^2
            let num = dy * p.x - dx * p.y + bx * ay - by * ax;
            let d = num * cs / len2;
            (t.clamp(0.0, 1.0), d)
        })
        .collect();

    pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    resample_profile(&pts, n)
}

fn resample_profile(pts: &[(f64, f64)], n: usize) -> Vec<f64> {
    if pts.is_empty() {
        return vec![0.0; n];
    }
    (0..n)
        .map(|i| {
            let t = if n <= 1 { 0.0 } else { i as f64 / (n - 1) as f64 };
            let idx = pts.partition_point(|x| x.0 < t);
            if idx == 0 {
                pts[0].1
            } else if idx >= pts.len() {
                pts.last().unwrap().1
            } else {
                let (t0, d0) = pts[idx - 1];
                let (t1, d1) = pts[idx];
                if (t1 - t0).abs() < 1e-10 {
                    d0
                } else {
                    d0 + (d1 - d0) * (t - t0) / (t1 - t0)
                }
            }
        })
        .collect()
}

/// Compare two signed profiles.
/// For a perfect match: pa[i] + pb[i] ≈ 0 (tab outward, hole inward cancel).
/// Tries both forward and reversed traversal of pb; takes the better direction.
///
/// Returns:
///   method1 = mean  |pa[i] + pb[j]|  (lower = better)
///   method2 = max   |pa[i] + pb[j]|  (Hausdorff, lower = better)
fn contour_diffs(pa: &[f64], pb: &[f64]) -> (f64, f64) {
    let n = pa.len();
    if n == 0 {
        return (0.0, 0.0);
    }

    let mut sum_fwd = 0.0f64;
    let mut sum_rev = 0.0f64;
    let mut max_fwd = 0.0f64;
    let mut max_rev = 0.0f64;

    for i in 0..n {
        let d_fwd = (pa[i] + pb[i]).abs();
        let d_rev = (pa[i] + pb[n - 1 - i]).abs();
        sum_fwd += d_fwd;
        sum_rev += d_rev;
        if d_fwd > max_fwd {
            max_fwd = d_fwd;
        }
        if d_rev > max_rev {
            max_rev = d_rev;
        }
    }

    let nf = n as f64;
    let m1 = (sum_fwd / nf).min(sum_rev / nf);
    let m2 = max_fwd.min(max_rev);
    (m1, m2)
}

// ─────────────────────────────────────────────────────────────────────────────
// Compatibility check
// ─────────────────────────────────────────────────────────────────────────────

/// Given that A's side `sa` connects to B's side `sb`, returns the clockwise
/// rotation (0/90/180/270) needed to orient B so that `sb` faces `sa`.
///
/// Derivation: rotating B by r° CW maps its sides as follows:
///   new_top = old_left(4), new_right = old_top(1), new_bottom = old_right(2), new_left = old_bottom(3)
/// So the original side of B that ends up opposite sa is determined by (sb - sa + 4) % 4:
///   diff 0 → 180°, diff 1 → 90°, diff 2 → 0°, diff 3 → 270°
fn rotation_for_sides(sa: SideIndex, sb: SideIndex) -> u16 {
    let diff = ((sb as i32 - sa as i32 + 4) % 4) as u8;
    match diff {
        0 => 180,
        1 => 90,
        2 => 0,
        3 => 270,
        _ => 0,
    }
}

fn types_compatible(a: &SideType, b: &SideType) -> bool {
    matches!(
        (a, b),
        (SideType::ConcaveInward, SideType::ConcaveOutward)
            | (SideType::ConcaveOutward, SideType::ConcaveInward)
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Scoring
// ─────────────────────────────────────────────────────────────────────────────

/// Compute a compatibility score [0..100] between two opposite-type sides.
fn compute_score(a: &PieceSide, b: &PieceSide, w: &MatchWeights) -> f64 {
    let ma = match &a.concavity {
        Some(m) => m,
        None => return 0.0,
    };
    let mb = match &b.concavity {
        Some(m) => m,
        None => return 0.0,
    };

    // ── 1. Euclidean distance similarity ─────────────────────────────────────
    // Compare: dist_to_corner_a of A  ↔  dist_to_corner_a of B  (and b side)
    // We try both orderings (a↔a and a↔b) and take the better one,
    // because the "corner_a" labelling might differ between pieces.
    let eu_score = {
        let s1 = similarity_ratio(ma.euclidean_dist_to_corner_a, mb.euclidean_dist_to_corner_a)
            * similarity_ratio(ma.euclidean_dist_to_corner_b, mb.euclidean_dist_to_corner_b);
        let s2 = similarity_ratio(ma.euclidean_dist_to_corner_a, mb.euclidean_dist_to_corner_b)
            * similarity_ratio(ma.euclidean_dist_to_corner_b, mb.euclidean_dist_to_corner_a);
        s1.max(s2)
    };

    // ── 2. Perimeter distance similarity ─────────────────────────────────────
    let pe_score = {
        let s1 = similarity_ratio(ma.perimeter_dist_to_corner_a, mb.perimeter_dist_to_corner_a)
            * similarity_ratio(ma.perimeter_dist_to_corner_b, mb.perimeter_dist_to_corner_b);
        let s2 = similarity_ratio(ma.perimeter_dist_to_corner_a, mb.perimeter_dist_to_corner_b)
            * similarity_ratio(ma.perimeter_dist_to_corner_b, mb.perimeter_dist_to_corner_a);
        s1.max(s2)
    };

    // ── 3. Depth similarity ───────────────────────────────────────────────────
    let depth_score = similarity_ratio(ma.depth, mb.depth);

    // ── 4. Apex position ratio similarity ────────────────────────────────────
    // The tab of A should sit at the same relative position as the hole of B.
    // Allow mirroring: pos_ratio ↔ (1 - pos_ratio)
    let pos_score = {
        let s1 = 1.0 - (ma.apex_position_ratio - mb.apex_position_ratio).abs();
        let s2 = 1.0 - (ma.apex_position_ratio - (1.0 - mb.apex_position_ratio)).abs();
        s1.max(s2).max(0.0)
    };

    // ── 5. Area similarity ────────────────────────────────────────────────────
    let area_score = similarity_ratio(ma.area, mb.area);

    // ── 6. Contour shape comparison ────────────────────────────────────────────
    let (contour_m1_score, contour_m2_score) = {
        let thr = w.contour_threshold;
        if a.centroid_side == 0.0
            || b.centroid_side == 0.0
            || a.contour.len() < 2
            || b.contour.len() < 2
            || thr < 1e-10
        {
            (1.0, 1.0) // no data available — no penalty
        } else {
            let pa = signed_profile(a, PROFILE_N);
            let pb = signed_profile(b, PROFILE_N);
            let (m1_diff, m2_diff) = contour_diffs(&pa, &pb);

            debug!(
                "  contour m1={:.4} m2={:.4} thr={:.4}",
                m1_diff, m2_diff, thr
            );

            // Hard gate: discard pair only if BOTH methods exceed threshold
            if m1_diff > thr && m2_diff > thr {
                return 0.0;
            }

            let s1 = (1.0 - m1_diff / thr).clamp(0.0, 1.0);
            let s2 = (1.0 - m2_diff / thr).clamp(0.0, 1.0);
            (s1, s2)
        }
    };

    // ── Weighted combination ──────────────────────────────────────────────────
    let total_weight = w.euclidean_weight
        + w.perimeter_weight
        + w.depth_weight
        + w.position_weight
        + w.area_weight
        + w.contour_mean_weight
        + w.contour_max_weight;
    let score = (eu_score * w.euclidean_weight
        + pe_score * w.perimeter_weight
        + depth_score * w.depth_weight
        + pos_score * w.position_weight
        + area_score * w.area_weight
        + contour_m1_score * w.contour_mean_weight
        + contour_m2_score * w.contour_max_weight)
        / total_weight;

    score * 100.0
}

/// Returns a similarity ratio in [0, 1] between two measurements.
/// 1.0 = identical, 0.0 = very different.
/// Uses the formula: min(a,b)/max(a,b), clamped to [0,1].
fn similarity_ratio(a: f64, b: f64) -> f64 {
    if a < 1e-6 && b < 1e-6 {
        return 1.0; // both essentially zero → identical
    }
    let max = a.max(b);
    let min = a.min(b);
    (min / max).clamp(0.0, 1.0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Cross-piece exclusion rule
// ─────────────────────────────────────────────────────────────────────────────

/// If piece A–sideX is matched with piece B–sideY, no other side of A can match
/// any side of B. Remove any such conflicting entries.
fn apply_exclusion_rule(output: &mut OutputMatches) {
    // Build a set of committed piece-pair exclusions from the best (first) match of each side.
    // "Best match" = first entry (already sorted by score desc).
    let mut excluded_pairs: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();

    for (from_key, matches) in &output.matches {
        if let Some(best) = matches.first() {
            let from_piece = piece_id_from_key(from_key);
            let to_piece = piece_id_from_key(&best.to_key);
            // Exclude in both directions
            let pair = canonical_pair(&from_piece, &to_piece);
            excluded_pairs.insert(pair);
        }
    }

    // Now remove any match that would violate a different side of the same piece-pair
    let keys: Vec<String> = output.matches.keys().cloned().collect();
    for from_key in &keys {
        let from_piece = piece_id_from_key(from_key);
        if let Some(matches) = output.matches.get_mut(from_key) {
            // The best match defines the committed pair
            if let Some(best) = matches.first().cloned() {
                let committed_to_piece = piece_id_from_key(&best.to_key);
                // Remove other matches that point to pieces that are excluded
                matches.retain(|m| {
                    let to_piece = piece_id_from_key(&m.to_key);
                    if to_piece == committed_to_piece {
                        // Same piece as committed → keep only the best (first)
                        m.to_key == best.to_key
                    } else {
                        // Different piece — check if this pair is excluded
                        let pair = canonical_pair(&from_piece, &to_piece);
                        !excluded_pairs.contains(&pair)
                    }
                });
            }
        }
    }
}

pub fn piece_id_from_key(key: &str) -> String {
    // key format: "000001-2"  → piece id "000001"
    key.rsplitn(2, '-').last().unwrap_or(key).to_string()
}

/// Confirm that piece_a connects to piece_b, working side by side.
///
/// For each side of piece_a that has at least one match to piece_b:
///   keep only the matches to piece_b for that side (remove matches to other pieces).
/// For sides of piece_a with no match to piece_b: leave untouched.
/// Same logic applied symmetrically to piece_b's sides.
///
/// Returns false (and does nothing) if no association between the two exists at all.
pub fn confirm_pair(output: &mut OutputMatches, piece_a: &str, piece_b: &str) -> bool {
    let exists = output.matches.iter().any(|(from_key, matches)| {
        let fp = piece_id_from_key(from_key);
        (fp == piece_a && matches.iter().any(|m| piece_id_from_key(&m.to_key) == piece_b))
            || (fp == piece_b && matches.iter().any(|m| piece_id_from_key(&m.to_key) == piece_a))
    });

    if !exists {
        return false;
    }

    let keys: Vec<String> = output.matches.keys().cloned().collect();
    for key in &keys {
        let fp = piece_id_from_key(key);
        if fp != piece_a && fp != piece_b {
            continue;
        }
        let partner = if fp == piece_a { piece_b } else { piece_a };

        // Only modify this side if it has at least one match to the partner piece.
        let has_match_to_partner = output
            .matches
            .get(key.as_str())
            .map(|v| v.iter().any(|m| piece_id_from_key(&m.to_key) == partner))
            .unwrap_or(false);

        if !has_match_to_partner {
            continue;
        }

        if let Some(v) = output.matches.get_mut(key.as_str()) {
            v.retain(|m| piece_id_from_key(&m.to_key) == partner);
        }
        if output.matches.get(key.as_str()).map(|v| v.is_empty()).unwrap_or(false) {
            output.matches.remove(key.as_str());
        }
    }

    true
}

fn canonical_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}
