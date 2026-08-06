# Module: duplicates

Purpose: find true duplicate files, recommend which copy to keep, quarantine the rest.

## Pipeline (staged to avoid hashing everything)
1. Walk user-selected roots (default: Downloads, Documents, Pictures, Videos, Desktop; folder picker to add). Skip: hidden/system attrs, protected paths, files < 1 MB (setting), quarantine dir.
2. Group by exact size → discard singletons.
3. Same-size groups: hash first 4 KB + last 4 KB (blake3) → subdivide.
4. Survivors: full blake3 hash (rayon parallel, progress events) → final duplicate sets.
5. Hardlink/junction awareness: same file-id (volume serial + file index via `windows` crate) = not a duplicate, exclude with note.

## Keep heuristic (brain-assisted)
Score each copy: prefer older `created` (original), path depth in organized dirs (Pictures/2023/ beats Downloads/), name without ` (1)`/`- Copy` suffixes, folder with more sibling structure. Highest score = suggested keeper; UI lets user flip per-set or "keep newest/oldest/in-folder-X" bulk switch. All removals = NeedsReview bucket minimum — duplicates are never auto-Safe.

## UI
Sets list sorted by wasted bytes desc; each set expandable: every copy with path, dates, keeper badge. Header: "X GB wasted in Y duplicate sets". Filters: by extension class (images/video/docs/other), min size.

## Acceptance
- Fixture with crafted collisions (same size, different content) → no false positives (stage 3/4 must catch).
- Hardlinked pair → excluded, reported as such.
- 10k-file fixture scan completes with progress events and cancellation working.
