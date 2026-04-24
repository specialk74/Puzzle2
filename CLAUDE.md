# Puzzle Solver — Project Context

## What this project does
Rust CLI that analyses puzzle piece images (JPEG/PNG), extracts geometric descriptors for each side, and computes which sides of which pieces are likely to connect. Results are saved to `output/output.json`.

## Build
```
cargo build          # debug
cargo build --release
```

## Run
```
puzzle --input ./input --output ./output --threshold 80
```
See `PARAMETERS.md` for full parameter documentation.

## Project structure
- `src/main.rs` — CLI parsing, orchestration, interactive confirmation loop
- `src/analysis.rs` — image processing: contour extraction, corner detection (`find_corners`), side classification (`classify_side`), debug image rendering
- `src/matching.rs` — pairwise side scoring (`compute_score`), exclusion rules, `confirm_pair`
- `src/models.rs` — data structures: `PieceDescriptor`, `PieceSide`, `ConcavityMetrics`, `SideMatch`, `OutputMatches`, `MatchWeights`
- `src/io.rs` — image loading, descriptor JSON cache, `output.json` read/write

## Key conventions

### Side indexing
1 = Top, 2 = Right, 3 = Bottom, 4 = Left (use `models::side_name(idx)` for labels).  
Internally in `analysis.rs` use the `Sides` enum (Top/Right/Bottom/Left) — never raw numbers in logs.

### Corner ordering
`find_corners` returns `[TL, TR, BR, BL]` (clockwise from top-left).  
`build_sides` maps them: TL→TR = Top, TR→BR = Right, BR→BL = Bottom, BL→TL = Left.

### Side types
- `SideType::Linear` — flat border edge, never matched
- `SideType::ConcaveInward` — hole (socket)
- `SideType::ConcaveOutward` — tab (knob)
Only Inward↔Outward pairs are compared.

### Perpendicular distance sign convention
`centroid_side` (+1 or -1) is stored in `PieceSide`.  
`d * centroid_side > 0` means the point is **inward** (toward the piece centroid).  
Tab apex → negative d; Hole apex → positive d. For a perfect match: `d_tab[t] + d_hole[t] ≈ 0`.

### Scoring
`compute_score` returns 0..100. Seven weighted components (all normalized by `total_weight`):
1. Euclidean corner distance similarity
2. Perimeter corner distance similarity
3. Concavity depth similarity
4. Apex position ratio similarity
5. Concavity area similarity (Shoelace)
6. Contour mean-diff score (Method 1)
7. Contour max-diff score / Hausdorff (Method 2)

Contour gate: if **both** Method 1 and Method 2 exceed `contour_threshold`, the pair is discarded (returns 0).

### output.json
Always recomputed from scratch on each run — never merged with a previous run.  
Piece descriptor JSONs (`{id}.json`) in the output directory **are** cached and reused unless deleted.

### Rotation
`rotation_for_sides(sa, sb)` returns the CW rotation (0/90/180/270°) to apply to the `to` piece so its side faces the `from` side.

## Dependencies worth knowing
- `opencv 0.98` with `clang-runtime` + `imgproc` features
- `rayon` for parallel piece analysis
- `fern` + `chrono` for dual-output logging (stderr + `output/puzzle.log`)
- `crossterm` for the interactive raw-terminal confirmation loop
- `clap` (derive) for CLI
- `anyhow` for error handling
- `serde` / `serde_json` for all JSON I/O

## Interactive loop (after analysis)
- Type a single number → shows all 4 sides of that piece with type (`Tab`/`Hole`/`Linear`) and connected piece numbers (`0` = no connection)
- Type two numbers (e.g. `1 2`) → confirms that piece 1 and piece 2 connect; keeps only the relevant side matches for those two pieces, removes alternatives
- `Esc` → exits
