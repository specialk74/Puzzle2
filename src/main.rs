mod analysis;
mod io;
mod matching;
mod models;

use anyhow::{Context, Result};
use clap::Parser;
use log::{error, info, warn};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

use models::{MatchWeights, PieceDescriptor};

// ─────────────────────────────────────────────────────────────────────────────
// CLI definition
// ─────────────────────────────────────────────────────────────────────────────

/// Puzzle solver: analyses puzzle piece images and finds side correspondences.
#[derive(Parser, Debug)]
#[command(
    name = "puzzle",
    version,
    about = "Analyses puzzle piece images and computes side correspondences"
)]
struct Cli {
    /// Directory containing puzzle piece images (input)
    #[arg(short, long, default_value = "input")]
    input: PathBuf,

    /// Directory for debug images and JSON descriptors (output)
    #[arg(short, long, default_value = "output")]
    output: PathBuf,

    /// Minimum compatibility score to include in output.json (0..100)
    #[arg(short, long, default_value_t = 80.0)]
    threshold: f64,

    /// Weight for euclidean corner-distance similarity (0..1)
    #[arg(long, default_value_t = 0.10)]
    weight_euclidean: f64,

    /// Weight for perimeter corner-distance similarity (0..1)
    #[arg(long, default_value_t = 0.10)]
    weight_perimeter: f64,

    /// Weight for concavity depth similarity (0..1)
    #[arg(long, default_value_t = 0.10)]
    weight_depth: f64,

    /// Weight for apex position ratio similarity (0..1)
    #[arg(long, default_value_t = 0.10)]
    weight_position: f64,

    /// Weight for concavity area similarity (0..1)
    #[arg(long, default_value_t = 0.50)]
    weight_area: f64,

    /// Weight for contour mean-diff score / Method 1 (0..1)
    #[arg(long, default_value_t = 0.10)]
    weight_contour_mean: f64,

    /// Weight for contour max-diff score / Method 2 Hausdorff (0..1)
    #[arg(long, default_value_t = 0.10)]
    weight_contour_max: f64,

    /// Relative contour-matching threshold (0..1, normalized by baseline length)
    #[arg(long, default_value_t = 0.15)]
    contour_threshold: f64,

    /// Number of threads to use for parallel image analysis (0 = use all CPUs)
    #[arg(long, default_value_t = 0)]
    threads: usize,

    /// Log level: error | warn | info | debug | trace
    #[arg(long, default_value = "info")]
    log_level: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

fn init_logger(level_str: &str, output_dir: &Path) -> Result<()> {
    let level = level_str
        .parse::<log::LevelFilter>()
        .unwrap_or(log::LevelFilter::Info);

    let log_path = output_dir.join("puzzle.log");
    let log_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&log_path)
        .with_context(|| format!("Cannot open log file {}", log_path.display()))?;

    let make_fmt = || {
        |out: fern::FormatCallback, message: &std::fmt::Arguments, record: &log::Record| {
            out.finish(format_args!(
                "[{}] [{:<5}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                record.level(),
                message
            ))
        }
    };

    fern::Dispatch::new()
        .level(level)
        .chain(
            fern::Dispatch::new()
                .format(make_fmt())
                .chain(std::io::stderr()),
        )
        .chain(fern::Dispatch::new().format(make_fmt()).chain(log_file))
        .apply()
        .context("Failed to initialise logger")?;

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Create output directory early so the log file can be opened there
    std::fs::create_dir_all(&cli.output)
        .with_context(|| format!("Cannot create output dir {}", cli.output.display()))?;

    init_logger(&cli.log_level, &cli.output)?;

    info!("═══════════════════════════════════════════════════");
    info!("  Puzzle Solver v{}", env!("CARGO_PKG_VERSION"));
    info!("═══════════════════════════════════════════════════");
    info!("Input  directory : {}", cli.input.display());
    info!("Output directory : {}", cli.output.display());
    info!("Threshold        : {:.1}%", cli.threshold);
    info!(
        "Weights — euclidean={} perimeter={} depth={} position={} area={} contour_mean={} contour_max={}",
        cli.weight_euclidean, cli.weight_perimeter, cli.weight_depth,
        cli.weight_position, cli.weight_area,
        cli.weight_contour_mean, cli.weight_contour_max
    );
    info!("Contour threshold : {:.3}", cli.contour_threshold);

    // Configure thread pool
    if cli.threads > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(cli.threads)
            .build_global()
            .context("Failed to configure Rayon thread pool")?;
        info!("Thread pool     : {} threads", cli.threads);
    } else {
        info!(
            "Thread pool     : {} threads (all CPUs)",
            rayon::current_num_threads()
        );
    }

    // Build match weights
    let weights = MatchWeights {
        euclidean_weight: cli.weight_euclidean,
        perimeter_weight: cli.weight_perimeter,
        depth_weight: cli.weight_depth,
        position_weight: cli.weight_position,
        area_weight: cli.weight_area,
        contour_mean_weight: cli.weight_contour_mean,
        contour_max_weight: cli.weight_contour_max,
        contour_threshold: cli.contour_threshold,
    };

    // Ensure input directory exists
    std::fs::create_dir_all(&cli.input)
        .with_context(|| format!("Cannot create input dir {}", cli.input.display()))?;

    // ── Phase 1: (skipped — output.json always recomputed from scratch) ────────
    let mut existing_matches = models::OutputMatches::default();

    // ── Phase 2: Discover input images ──────────────────────────────────────
    info!("─── Phase 2: Discovering images ───");
    let images = io::discover_images(&cli.input)?;

    if images.is_empty() {
        warn!("No images found in {}. Nothing to do.", cli.input.display());
        return Ok(());
    }
    info!("Found {} images", images.len());

    // ── Phase 3: Parallel image analysis ────────────────────────────────────
    info!("─── Phase 3: Analysing pieces (parallel) ───");

    // Collect results from parallel workers
    let results: Vec<Result<PieceDescriptor>> = images
        .par_iter()
        .map(|(id, path)| -> Result<PieceDescriptor> {
            // Try to load cached descriptor first
            if let Some(cached) = io::load_piece_descriptor(id, &cli.output)? {
                info!("[{}] Using cached descriptor (skipping analysis)", id);
                return Ok(cached);
            }

            // Load and analyse the image
            info!("[{}] Loading image from {}", id, path.display());
            let img = io::load_image(path)
                .with_context(|| format!("Cannot load image for piece '{}'", id))?;

            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(id)
                .to_string();

            let descriptor = analysis::analyse_piece(&cli.output, &img, id, &filename)
                .with_context(|| format!("Analysis failed for piece '{}'", id))?;

            // Save descriptor JSON
            io::save_piece_descriptor(&descriptor, &cli.output)?;

            // Save debug image with the same name as input (but in output dir)
            let debug_img = analysis::render_debug_image(&img, &descriptor);
            io::save_debug_image(&debug_img, &cli.output, &filename)?;

            Ok(descriptor)
        })
        .collect();

    // Separate successes from failures
    let mut pieces: Vec<PieceDescriptor> = Vec::new();
    let mut failed = 0usize;

    for result in results {
        match result {
            Ok(desc) => pieces.push(desc),
            Err(e) => {
                error!("Piece analysis error: {:#}", e);
                failed += 1;
            }
        }
    }

    info!("Analysis complete: {} OK, {} failed", pieces.len(), failed);

    if pieces.is_empty() {
        warn!("No pieces analysed successfully. Exiting.");
        return Ok(());
    }

    // ── Phase 4: Matching ────────────────────────────────────────────────────
    info!("─── Phase 4: Computing side matches ───");

    let new_matches = matching::compute_matches(&pieces, &weights, cli.threshold);

    // Merge with existing matches
    matching::merge_matches(&mut existing_matches, new_matches);

    // ── Phase 5: Save output.json ────────────────────────────────────────────
    info!("─── Phase 5: Saving results ───");
    io::save_output_matches(&existing_matches, &cli.output)?;

    // Print human-readable summary
    let summary = io::format_matches_human(&existing_matches);
    info!("─── Match summary ───");
    if summary.is_empty() {
        info!("(no matches above {:.0}% threshold)", cli.threshold);
    } else {
        for line in summary.lines() {
            info!("  {}", line);
        }
    }

    info!("═══════════════════════════════════════════════════");
    info!("  Done! Results in {}", cli.output.display());
    info!("═══════════════════════════════════════════════════");

    // ── Replay coppie utente da user.json ────────────────────────────────────
    info!("─── Replay coppie utente ───");
    let user_pairs = io::load_user_pairs(&cli.output)?;
    let mut replay_changed = false;
    for (pa, pb) in &user_pairs.confirmed_pairs {
        if matching::apply_user_pair(&mut existing_matches, pa, pb) {
            info!("Replayed pair: {} ↔ {}", pa, pb);
            replay_changed = true;
        } else {
            warn!("Coppia {} ↔ {} non trovata nei match correnti, ignorata", pa, pb);
        }
    }
    if replay_changed {
        io::save_output_matches(&existing_matches, &cli.output)?;
    }

    // ── Phase 6: Interactive confirmation ───────────────────────────────────
    interactive_loop(&cli.output, user_pairs)?;

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Interactive confirmation loop
// ─────────────────────────────────────────────────────────────────────────────

fn interactive_loop(output_dir: &Path, user_pairs: models::UserPairs) -> Result<()> {
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

    println!("─── Interactive confirmation ───");
    println!("Enter a number to inspect a piece, or two numbers to confirm a match (e.g. '1 2'). Press Esc to exit.");

    enable_raw_mode()?;
    let result = run_interactive_loop(output_dir, user_pairs);
    disable_raw_mode()?;
    println!();
    result
}

fn run_interactive_loop(output_dir: &Path, mut user_pairs: models::UserPairs) -> Result<()> {
    use crossterm::cursor;
    use crossterm::event::{read, Event, KeyCode};
    use crossterm::execute;
    use crossterm::terminal::{Clear, ClearType};
    use std::io::{stdout, Write};

    let mut buf = String::new();

    loop {
        execute!(
            stdout(),
            cursor::MoveToColumn(0),
            Clear(ClearType::CurrentLine)
        )?;
        print!("> {}", buf);
        stdout().flush()?;

        match read()? {
            Event::Key(key) => match key.code {
                KeyCode::Esc => {
                    info!("Interactive loop exited by user.");
                    break;
                }

                KeyCode::Enter => {
                    print!("\r\n");
                    stdout().flush()?;

                    let parts: Vec<String> =
                        buf.trim().split_whitespace().map(str::to_owned).collect();
                    buf.clear();

                    // ── Singolo numero: mostra info pezzo ────────────────────
                    if parts.len() == 1 {
                        match parts[0].parse::<u32>() {
                            Ok(a) => {
                                let piece_id = format!("{:06}", a);
                                let output_matches = io::load_output_matches(output_dir)?;
                                print_piece_info(&output_matches, &piece_id, &user_pairs, output_dir)?;
                            }
                            Err(_) => {
                                print!("Invalid number.\r\n");
                                stdout().flush()?;
                            }
                        }
                        continue;
                    }

                    if parts.len() != 2 {
                        if !parts.is_empty() {
                            print!("Expected 1 or 2 numbers.\r\n");
                            stdout().flush()?;
                        }
                        continue;
                    }

                    // ── Due numeri: conferma coppia ───────────────────────────
                    match (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                        (Ok(a), Ok(b)) => {
                            let piece_a = format!("{:06}", a);
                            let piece_b = format!("{:06}", b);

                            let mut output_matches = io::load_output_matches(output_dir)?;
                            if matching::apply_user_pair(&mut output_matches, &piece_a, &piece_b) {
                                io::save_output_matches(&output_matches, output_dir)?;
                                user_pairs.add(&piece_a, &piece_b);
                                io::save_user_pairs(&user_pairs, output_dir)?;
                                info!("Confirmed pair: {} ↔ {}", piece_a, piece_b);
                                print_piece_info(&output_matches, &piece_a, &user_pairs, output_dir)?;
                                print_piece_info(&output_matches, &piece_b, &user_pairs, output_dir)?;
                            } else {
                                print!(
                                    "No association found between {} and {}.\r\n",
                                    piece_a, piece_b
                                );
                                stdout().flush()?;
                            }
                        }
                        _ => {
                            print!("Invalid input: expected two integers.\r\n");
                            stdout().flush()?;
                        }
                    }
                }

                KeyCode::Char(c) => {
                    buf.push(c);
                }

                KeyCode::Backspace => {
                    buf.pop();
                }

                _ => {}
            },
            _ => {}
        }
    }

    Ok(())
}

fn print_piece_info(
    output_matches: &models::OutputMatches,
    piece_id: &str,
    user_pairs: &models::UserPairs,
    output_dir: &Path,
) -> Result<()> {
    use std::io::Write;

    let n: u32 = piece_id.trim_start_matches('0').parse().unwrap_or(0);
    let descriptor = io::load_piece_descriptor(piece_id, output_dir)?;

    // Mappa side_idx → lista di stringhe (con ANSI bold per i confermati)
    let mut targets_by_idx: std::collections::BTreeMap<u8, Vec<String>> =
        std::collections::BTreeMap::new();
    for (k, v) in output_matches.matches.iter() {
        if matching::piece_id_from_key(k) != piece_id {
            continue;
        }
        let side_idx: u8 = k
            .rsplitn(2, '-')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        // Deduplica per numero pezzo: accumula i flag di stile con OR
        let mut seen: std::collections::HashMap<u32, (bool, bool)> =
            std::collections::HashMap::new();
        let mut order: Vec<u32> = Vec::new();
        for m in v.iter() {
            let pid = matching::piece_id_from_key(&m.to_key);
            let num: u32 = pid.trim_start_matches('0').parse().unwrap_or(0);
            let confirmed = user_pairs.contains(piece_id, &pid);
            let mutual = matching::is_mutual(output_matches, k, &m.to_key);
            if let Some(entry) = seen.get_mut(&num) {
                entry.0 |= confirmed;
                entry.1 |= mutual;
            } else {
                seen.insert(num, (confirmed, mutual));
                order.push(num);
            }
        }
        let nums: Vec<String> = order
            .iter()
            .map(|num| {
                let (confirmed, mutual) = seen[num];
                match (confirmed, mutual) {
                    (true, true)  => format!("\x1b[1m\x1b[4m{}\x1b[0m", num),
                    (true, false) => format!("\x1b[1m{}\x1b[0m", num),
                    (false, true) => format!("\x1b[4m{}\x1b[0m", num),
                    (false, false) => num.to_string(),
                }
            })
            .collect();
        targets_by_idx.insert(side_idx, nums);
    }

    let type_label = |st: &models::SideType| match st {
        models::SideType::ConcaveInward => "Hole",
        models::SideType::ConcaveOutward => "Tab",
        models::SideType::Linear => "Linear",
    };

    print!("\r\nPiece {}:\r\n", n);

    if let Some(desc) = &descriptor {
        for side in &desc.sides {
            let name = models::side_name(side.index);
            let label = type_label(&side.side_type);
            match targets_by_idx.get(&side.index) {
                Some(ts) => print!("  {}({}) → {}\r\n", name, label, ts.join(", ")),
                None => print!("  {}({}) → 0\r\n", name, label),
            }
        }
    } else {
        if targets_by_idx.is_empty() {
            print!("  (no connections)\r\n");
        }
        for (idx, ts) in &targets_by_idx {
            let name = models::side_name(*idx);
            print!("  {} → {}\r\n", name, ts.join(", "));
        }
    }

    std::io::stdout().flush()?;
    Ok(())
}
