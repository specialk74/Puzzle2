use anyhow::{Context, Result};
use image::{DynamicImage, RgbImage};
use log::{debug, info};
use std::path::{Path, PathBuf};

use crate::models::{OutputMatches, PieceDescriptor, UserPairs};

// ─────────────────────────────────────────────────────────────────────────────
// Piece descriptor (per-piece JSON)
// ─────────────────────────────────────────────────────────────────────────────

/// Save a piece descriptor to <output_dir>/<id>.json
pub fn save_piece_descriptor(
    descriptor: &PieceDescriptor,
    output_dir: &Path,
) -> Result<()> {
    let path = output_dir.join(format!("{}.json", descriptor.id));
    let json = serde_json::to_string_pretty(descriptor)
        .context("Failed to serialise piece descriptor")?;
    std::fs::write(&path, json)
        .with_context(|| format!("Failed to write descriptor to {}", path.display()))?;
    debug!("Saved descriptor to {}", path.display());
    Ok(())
}

/// Load a piece descriptor from <output_dir>/<id>.json — returns None if not found.
pub fn load_piece_descriptor(id: &str, output_dir: &Path) -> Result<Option<PieceDescriptor>> {
    let path = output_dir.join(format!("{}.json", id));
    if !path.exists() {
        return Ok(None);
    }
    let json = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let desc: PieceDescriptor =
        serde_json::from_str(&json)
            .with_context(|| format!("Failed to parse descriptor at {}", path.display()))?;
    info!("Loaded cached descriptor for piece '{}'", id);
    Ok(Some(desc))
}

// ─────────────────────────────────────────────────────────────────────────────
// Output matches (output.json)
// ─────────────────────────────────────────────────────────────────────────────

/// Save the global matches file to <output_dir>/output.json
pub fn save_output_matches(matches: &OutputMatches, output_dir: &Path) -> Result<()> {
    let path = output_dir.join("output.json");
    let json = serde_json::to_string_pretty(matches)
        .context("Failed to serialise output matches")?;
    std::fs::write(&path, &json)
        .with_context(|| format!("Failed to write output.json to {}", path.display()))?;
    info!("Saved output.json to {}", path.display());
    Ok(())
}

/// Load the global matches file from <output_dir>/output.json — returns empty if not found.
pub fn load_output_matches(output_dir: &Path) -> Result<OutputMatches> {
    let path = output_dir.join("output.json");
    if !path.exists() {
        info!("output.json not found, starting fresh");
        return Ok(OutputMatches::default());
    }
    let json = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read output.json from {}", path.display()))?;
    let matches: OutputMatches = serde_json::from_str(&json)
        .context("Failed to parse output.json")?;
    info!(
        "Loaded output.json: {} side keys already matched",
        matches.matches.len()
    );
    Ok(matches)
}

// ─────────────────────────────────────────────────────────────────────────────
// Image file discovery
// ─────────────────────────────────────────────────────────────────────────────

/// Supported image extensions
const SUPPORTED_EXT: &[&str] = &["jpg", "jpeg", "png", "bmp", "tiff", "tif", "webp"];

/// Discover all puzzle piece images in the input directory.
/// Returns a list of (id, path) tuples, sorted by filename stem.
pub fn discover_images(input_dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut entries: Vec<(String, PathBuf)> = std::fs::read_dir(input_dir)
        .with_context(|| format!("Cannot open input directory {}", input_dir.display()))?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    if SUPPORTED_EXT.contains(&ext.to_lowercase().as_str()) {
                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                            return Some((stem.to_string(), path));
                        }
                    }
                }
            }
            None
        })
        .collect();

    entries.sort_by(|a, b| a.0.cmp(&b.0));
    info!("Discovered {} images in {}", entries.len(), input_dir.display());
    Ok(entries)
}

/// Load an image from disk.
pub fn load_image(path: &Path) -> Result<DynamicImage> {
    image::open(path).with_context(|| format!("Failed to open image {}", path.display()))
}

/// Save a debug RGB image to <output_dir>/<filename>
pub fn save_debug_image(
    img: &RgbImage,
    output_dir: &Path,
    filename: &str,
) -> Result<()> {
    let path = output_dir.join(filename);
    img.save(&path)
        .with_context(|| format!("Failed to save debug image to {}", path.display()))?;
    debug!("Saved debug image to {}", path.display());
    Ok(())
}


// ─────────────────────────────────────────────────────────────────────────────
// User pairs (user.json)
// ─────────────────────────────────────────────────────────────────────────────

/// Salva le coppie confermate dall'utente in <output_dir>/user.json
pub fn save_user_pairs(pairs: &UserPairs, output_dir: &Path) -> Result<()> {
    let path = output_dir.join("user.json");
    let json = serde_json::to_string_pretty(pairs)
        .context("Failed to serialise user pairs")?;
    std::fs::write(&path, json)
        .with_context(|| format!("Failed to write user.json to {}", path.display()))?;
    debug!("Saved user.json to {}", path.display());
    Ok(())
}

/// Carica le coppie confermate da <output_dir>/user.json — ritorna vuoto se non esiste.
pub fn load_user_pairs(output_dir: &Path) -> Result<UserPairs> {
    let path = output_dir.join("user.json");
    if !path.exists() {
        return Ok(UserPairs::default());
    }
    let json = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read user.json from {}", path.display()))?;
    serde_json::from_str(&json).context("Failed to parse user.json")
}

// ─────────────────────────────────────────────────────────────────────────────
// Output formatting for human-readable display
// ─────────────────────────────────────────────────────────────────────────────

/// Format matches in the requested human-readable style:
/// 000002-2 -> 000001-1|85.0%, 000003-3|80.0%
pub fn format_matches_human(matches: &OutputMatches) -> String {
    let mut lines: Vec<String> = Vec::new();
    for (from_key, side_matches) in &matches.matches {
        let rhs: Vec<String> = side_matches
            .iter()
            .map(|m| format!("{}|{:.1}%|rot{}°", m.to_key, m.score, m.rotation))
            .collect();
        lines.push(format!("{} -> {}", from_key, rhs.join(", ")));
    }
    lines.join("\n")
}
