<!-- durum: aktif -->
# Architecture

Tauri v2 desktop app. Rust does all filesystem/registry/system work; the WebView (Svelte 5 + TS + Tailwind) only renders and collects consent.

## 1. Repository layout

```
/
├── CLAUDE.md
├── docs/                     # specs (this folder)
├── rules/                    # brain rule packs, JSON (shipped with app, community-editable)
├── src/                      # Svelte UI
│   ├── lib/
│   │   ├── api.ts            # typed wrappers around invoke() — ONLY place invoke is called
│   │   ├── stores/           # scan state, settings, quarantine
│   │   └── components/       # shared: RiskBadge, FindingList, ProgressBar, ConfirmDialog
│   └── routes/               # one folder per tool module (lazy-loaded)
│       ├── dashboard/
│       ├── cleaner/
│       ├── treemap/
│       ├── duplicates/
│       ├── startup/
│       └── shutdown/
└── src-tauri/
    ├── src/
    │   ├── main.rs           # one-line shim calling the lib crate's run()
    │   ├── lib.rs            # tauri builder, command registration only (v2 layout)
    │   ├── commands.rs       # #[tauri::command] fns — thin, delegate to core
    │   ├── core/
    │   │   ├── module.rs     # ToolModule trait + registry
    │   │   ├── brain/        # rule engine, scoring, protected paths, llm/ (optional providers)
    │   │   ├── scanner.rs    # shared parallel walker (jwalk + rayon)
    │   │   ├── executor.rs   # applies approved recommendations via quarantine
    │   │   ├── quarantine.rs
    │   │   ├── db.rs         # rusqlite, migrations
    │   │   └── events.rs     # typed event emission to UI
    │   └── modules/
    │       ├── cleaner.rs
    │       ├── treemap.rs
    │       ├── duplicates.rs
    │       ├── startup.rs
    │       └── shutdown_timer.rs
    └── Cargo.toml
```

Single Rust crate for V1 (split into workspace crates only if compile times hurt). Every new tool = one file in `modules/` + one folder in `routes/` + registry entry. Nothing else changes.

## 2. Module contract (the heart of the design)

```rust
// core/module.rs — spec-level sketch; implementer finalizes signatures
pub trait ToolModule: Send + Sync {
    fn id(&self) -> &'static str;                 // "cleaner", "shutdown-timer"...
    fn manifest(&self) -> ModuleManifest;         // name, icon, description, capabilities
    fn scan(&self, ctx: &ScanContext) -> Result<Vec<Finding>>;       // read-only, emits progress events
    fn analyze(&self, findings: &[Finding], brain: &Brain) -> Vec<Recommendation>;
    fn execute(&self, approved: &[Uuid], ctx: &ExecContext) -> Result<ExecReport>; // via executor/quarantine only
}
```

Key types (serde-serializable, shared with TS via generated types or hand-mirrored in `api.ts`):

- **Finding** — `id: Uuid, module: String, path: PathBuf, kind: FindingKind, size_bytes: u64, created/modified/accessed: Option<DateTime>, meta: serde_json::Value`
- **Recommendation** — `id, finding_ids: Vec<Uuid>, risk: RiskBucket, confidence: u8 (0–100), action: Action, reason: String, savings_bytes: u64`
- **RiskBucket** — `Safe | NeedsReview | DoNotDelete` (UI: green / yellow / red)
- **Action** — `Quarantine | DisableStartupEntry | Ignore | InfoOnly` (note: no raw `Delete`)
- **ExecReport** — per-item success/failure, total reclaimed bytes, quarantine batch id

Modules that aren't about files (shutdown timer) implement `scan/analyze` as no-ops and expose module-specific commands; the trait still gives them registry presence, manifest, and settings storage.

## 3. IPC & events

- Commands (`commands.rs`): `list_modules`, `start_scan(module_id)`, `cancel_scan(scan_id)`, `get_recommendations(scan_id)`, `execute(scan_id, approved_ids)`, `quarantine_list/restore/purge`, plus module-specific ones (e.g. `shutdown_schedule`).
- Events (Rust → UI): `scan:progress {scan_id, module, items_scanned, current_path, pct?}`, `scan:done {scan_id, finding_count}`, `exec:progress`, `quarantine:changed`. Throttle progress to ≤10 events/sec.
- Long work runs on `tauri::async_runtime::spawn_blocking`; scans are cancellable via `CancellationToken` checked in walker loop.

## 4. Brain (hybrid intelligence)

**Layer 1 — rule engine (always on, offline).** Loads `rules/*.json` at startup (see `docs/RULES-FORMAT.md`). Scoring pipeline per finding:

1. Hard gates: protected path → force `DoNotDelete`, confidence 100, stop.
2. Rule match (path glob + extension + optional owner-app check): base score + bucket from rule.
3. Heuristic modifiers: last-access age (+), size (+ for review candidates), file locked by running process (→ `DoNotDelete`), inside known app cache dir (+).
4. Score → bucket thresholds: ≥85 Safe, 40–84 NeedsReview, <40 DoNotDelete. Rules may pin a bucket ceiling (a rule can never promote above its declared max bucket).

`protected_paths()` hard-coded in Rust: `%WINDIR%`, `Program Files`/`Program Files (x86)`, `%USERPROFILE%` root itself (not subdirs), `System Volume Information`, drive roots, quarantine dir, the app's own dirs.

**Layer 2 — LLM (optional, off by default).** `brain/llm/` defines `LlmProvider` trait with `explain(recommendation, metadata) -> String` and `classify(unknown_dir_metadata) -> SuggestedBucket`. Adapters: Anthropic, OpenAI, Ollama (local). Payload = path, size, dates, matched-rule id only — **never file contents**. API key stored via OS credential store (keyring crate), not plaintext settings. Every LLM feature must degrade gracefully to rule-engine-only.

## 5. Persistence (SQLite via rusqlite, file in app data dir)

- `scans(id, module, started_at, finished_at, finding_count, status)`
- `actions(id, scan_id, recommendation_json, executed_at, result)`
- `quarantine(id, batch_id, original_path, quarantine_path, size_bytes, quarantined_at, purge_after, restored_at NULL)`
- `settings(key, value)` — includes per-module settings JSON
- Migrations: numbered SQL files embedded via `include_str!`, applied at startup, version in `pragma user_version`.

## 6. Quarantine (the trust feature — build it first, build it well)

- Location: `{app_data}/quarantine/{batch_id}/{uuid}` — original path recorded in DB. Same-volume `rename` when possible; copy+verify+delete when crossing volumes.
- Default retention 7 days (setting), then purge on app start (with size cap safeguard: purge oldest first if quarantine exceeds configurable max GB).
- `restore(batch_id | item_id)` moves back to original path; on collision, restore alongside with ` (restored)` suffix and report it.
- Locked/permission-denied files: skip, report per-item error, never partial-delete a batch silently.

## 7. UI shell

Sidebar (module registry drives it — icons + names from manifests) → each route follows the same 3-step pattern where applicable: **Scan → Review (3 risk buckets, select items, every item shows `reason`) → Execute (confirm dialog shows total GB + "goes to quarantine, reversible for 7 days")**. Global elements: quarantine drawer with restore, scan progress toast, dark theme default. Design language: calm, minimal, no scare-ware red alerts — red bucket is informational, has no action button.

## 8. Error handling & logging

- `thiserror` for domain errors; commands return `Result<T, ApiError>` where `ApiError` serializes `{code, message, detail?}` for the UI.
- `tracing` + rolling file log in app data dir (this app deletes things — auditability matters). Every execute batch logs each item.

## 9. Testing strategy

- Unit: brain scoring (table-driven: path+metadata → expected bucket), rules parser (reject malformed packs loudly), quarantine round-trip (tempdir), duplicate hasher.
- Integration: fake filesystem fixtures under `tests/fixtures/` — a miniature dirty Windows profile the cleaner scans deterministically.
- CI (GitHub Actions, `windows-latest`): clippy + tests + tauri build; artifact = portable exe + NSIS installer on tag.
