# Module: treemap (disk visualization)

Purpose: answer "where did my disk space go?" visually; funnel discoveries into cleaner/duplicates.

## Backend
- `scan(root)` builds a directory-size tree: parallel walk, aggregate sizes bottom-up. Stream partial results (top 2 levels first, refine deeper) so UI paints early.
- Serve tree to UI in chunks: `treemap:node {path, size, child_count, depth}` events or paginated command `get_children(path)` — pick ONE: recommendation is lazy `get_children` (simpler, memory-safe); precompute full tree in Rust, UI fetches on zoom-in.
- Cache last scan per drive in SQLite (staleness timestamp shown in UI; rescan button).
- Handle: reparse points/junctions (don't follow — count once), permission-denied dirs (mark, size = unknown), NTFS compression (logical size; show note).

## UI
- Squarified treemap (implement in TS with d3-hierarchy or hand-rolled squarify — no heavy chart lib). Canvas rendering, not SVG (perf on 10k+ rects).
- Color by file category (video/image/audio/archive/code/app/other), depth-shaded. Hover: path + size. Click: zoom in; breadcrumb to zoom out.
- Right-click / action menu on a node: "Open in Explorer", "Scan with Cleaner", "Find duplicates here" (routes to those modules with path pre-filled — this cross-linking is the Swiss-knife glue, do not skip it).
- Drive selector; per-drive total/free bar.

## Non-goals (V1)
No MFT parsing (WizTree-level speed is a V2 stretch — note in DECISIONS-NEEDED if walk is too slow on HDDs), no file-level rects below 0.1% of view area (aggregate as "…").

## Acceptance
- Fixture tree renders; zoom in/out; sizes sum correctly vs `du`-style test oracle.
- 100k-file synthetic tree: first paint < 3s on dev machine, UI stays responsive (chunked).
- Cross-link buttons navigate with pre-filled path.
