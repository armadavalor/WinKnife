/**
 * Hand-mirrored counterparts of the serde types in
 * `src-tauri/src/core/types.rs`, `core/module.rs`, `core/events.rs` and
 * `core/error.rs`.
 *
 * When a Rust struct changes, change it here in the same commit
 * (CONVENTIONS.md). The Rust side has wire-format tests pinning the exact JSON
 * these interfaces describe.
 */

/* -------------------------------------------------------------------------
 * core/error.rs
 * ---------------------------------------------------------------------- */

/** What every failing command rejects with. */
export interface ApiError {
  /** Stable machine-readable code, e.g. `unknown_module`. */
  code: string;
  /** Human-readable English message, safe to show. */
  message: string;
  /** Extra context; absent rather than null when there is none. */
  detail?: string;
}

/** Narrow an unknown rejection value to an {@link ApiError}. */
export function isApiError(value: unknown): value is ApiError {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.code === "string" && typeof candidate.message === "string"
  );
}

/* -------------------------------------------------------------------------
 * core/types.rs
 * ---------------------------------------------------------------------- */

/** What a {@link Finding} points at. */
export type FindingKind =
  | "file"
  | "directory"
  | "recycleBinItem"
  | "registryEntry"
  | "scheduledTask"
  | "other";

/** Something a module found. Facts only — the opinion is in {@link Recommendation}. */
export interface Finding {
  /** UUID. */
  id: string;
  /** Id of the module that produced it. */
  module: string;
  /** Absolute path (or registry path). */
  path: string;
  kind: FindingKind;
  /** Size in bytes; `0` when the concept does not apply. */
  sizeBytes: number;
  /** RFC 3339 UTC timestamp, or null when the filesystem has none. */
  created: string | null;
  modified: string | null;
  accessed: string | null;
  /** Module-specific extras, e.g. the id of the rule that matched. */
  meta: unknown;
}

/** The three risk buckets: green / yellow / red. */
export type RiskBucket = "safe" | "needsReview" | "doNotDelete";

/**
 * What we propose doing. There is no raw `delete`: removal always means
 * "move to quarantine", which is reversible.
 */
export type ActionKind =
  | "quarantine"
  | "disableStartupEntry"
  | "ignore"
  | "infoOnly";

/** A suggestion about one or more findings. Always carries a `reason`. */
export interface Recommendation {
  id: string;
  findingIds: string[];
  risk: RiskBucket;
  /** 0-100. */
  confidence: number;
  action: ActionKind;
  /** Human-readable justification, shown next to every item. */
  reason: string;
  savingsBytes: number;
}

/** Outcome of applying one recommendation to one finding. */
export type ExecOutcome =
  | { status: "succeeded" }
  | { status: "skipped"; reason: string }
  | { status: "failed"; error: string };

/** Per-item result inside an {@link ExecReport}. */
export interface ExecItemResult {
  recommendationId: string;
  findingId: string;
  path: string;
  outcome: ExecOutcome;
  reclaimedBytes: number;
}

/** What an execute run actually did. */
export interface ExecReport {
  /** Quarantine batch the moved items landed in, or null. */
  batchId: string | null;
  items: ExecItemResult[];
  reclaimedBytes: number;
}

/* -------------------------------------------------------------------------
 * core/module.rs
 * ---------------------------------------------------------------------- */

/** What a module supports, so the UI can hide what does not apply. */
export interface ModuleCapabilities {
  hasScan: boolean;
  hasExecute: boolean;
  hasSettings: boolean;
}

/** Everything the UI needs to render a module without knowing anything else. */
export interface ModuleManifest {
  /** kebab-case; also the route segment (`/dummy`). */
  id: string;
  /** English display name; the i18n dictionary may override it. */
  name: string;
  description: string;
  /** Icon name the UI maps to an inline SVG. */
  icon: string;
  capabilities: ModuleCapabilities;
}

/* -------------------------------------------------------------------------
 * core/events.rs
 * ---------------------------------------------------------------------- */

/** Payload of `scan:progress`. */
export interface ScanProgress {
  scanId: string;
  module: string;
  itemsScanned: number;
  currentPath: string | null;
  /** 0-100 when it can be estimated at all. */
  pct: number | null;
}

/** Payload of `scan:done`. */
export interface ScanDone {
  scanId: string;
  module: string;
  findingCount: number;
}

/** Payload of `exec:progress`. */
export interface ExecProgress {
  scanId: string;
  module: string;
  itemsDone: number;
  itemsTotal: number;
  currentPath: string | null;
}

/** Payload of `quarantine:changed`. */
export interface QuarantineChanged {
  batchId: string | null;
  itemCount: number;
  totalBytes: number;
}

/** Payload of `shutdown:tick`. */
export interface ShutdownTick {
  remainingSecs: number;
}

/* -------------------------------------------------------------------------
 * core/state.rs
 * ---------------------------------------------------------------------- */

/** Everything one scan produced. */
export interface ScanResult {
  scanId: string;
  module: string;
  findings: Finding[];
  recommendations: Recommendation[];
  /** Rules that could not run, as `pack/rule: %VARIABLE%`. */
  skippedRules: string[];
}

/* -------------------------------------------------------------------------
 * modules/cleaner.rs
 * ---------------------------------------------------------------------- */

/** Metadata every cleaner finding carries, so the UI can group by pack. */
export interface FindingMeta {
  packId: string;
  packName: string;
  ruleId: string;
  /** Recycle-bin findings only. */
  itemCount?: number;
}

/** Totals for the header line after a scan. */
export interface CleanerSummary {
  safeBytes: number;
  reviewBytes: number;
  blockedItems: number;
}

/** Narrow a duplicate finding's `meta` to {@link DuplicateFindingMeta}. */
export function duplicateMeta(meta: unknown): DuplicateFindingMeta | null {
  if (typeof meta !== "object" || meta === null) {
    return null;
  }
  const candidate = meta as Record<string, unknown>;
  if (
    typeof candidate.setId !== "string" ||
    typeof candidate.contentHash !== "string" ||
    typeof candidate.keeper !== "boolean"
  ) {
    return null;
  }
  return {
    setId: candidate.setId,
    contentHash: candidate.contentHash,
    keeper: candidate.keeper,
    copiesInSet:
      typeof candidate.copiesInSet === "number"
        ? candidate.copiesInSet
        : undefined,
    keeperPath:
      typeof candidate.keeperPath === "string" ? candidate.keeperPath : undefined,
  };
}

/** Narrow a finding's `meta` to {@link FindingMeta}. */
export function findingMeta(meta: unknown): FindingMeta | null {
  if (typeof meta !== "object" || meta === null) {
    return null;
  }
  const candidate = meta as Record<string, unknown>;
  if (
    typeof candidate.packId !== "string" ||
    typeof candidate.packName !== "string" ||
    typeof candidate.ruleId !== "string"
  ) {
    return null;
  }
  return {
    packId: candidate.packId,
    packName: candidate.packName,
    ruleId: candidate.ruleId,
    itemCount:
      typeof candidate.itemCount === "number" ? candidate.itemCount : undefined,
  };
}

/* -------------------------------------------------------------------------
 * modules/duplicates.rs
 * ---------------------------------------------------------------------- */

/** One copy inside a {@link DuplicateSet}. */
export interface DuplicateCopy {
  findingId: string;
  path: string;
  /** RFC 3339 UTC, or null. */
  created: string | null;
  modified: string | null;
  /** Whether the heuristic suggests keeping this one. */
  keeper: boolean;
  keeperScore: number;
  keeperReason: string;
}

/** A set of files with identical content. */
export interface DuplicateSet {
  id: string;
  /** Hex blake3 of the content. */
  contentHash: string;
  /** Size of one copy. */
  sizeBytes: number;
  /** Every copy, keeper first. */
  copies: DuplicateCopy[];
  /** `sizeBytes * (copies - 1)`. */
  wastedBytes: number;
}

/** A name excluded because it is a second name for a file already in the set. */
export interface LinkedName {
  path: string;
  sameFileAs: string;
  note: string;
}

/** What one duplicate scan produced. */
export interface DuplicateScan {
  findings: Finding[];
  sets: DuplicateSet[];
  linkedNames: LinkedName[];
  skippedRoots: string[];
  filesHashed: number;
  cancelled: boolean;
}

/** What `start_duplicate_scan` hands back. */
export interface DuplicateScanResult {
  scanId: string;
  scan: DuplicateScan;
  /** One per copy. Never `safe`. */
  recommendations: Recommendation[];
  wastedBytes: number;
}

/** Metadata every duplicate finding carries. */
export interface DuplicateFindingMeta {
  setId: string;
  contentHash: string;
  keeper: boolean;
  /** How many copies the set holds, including the keeper. */
  copiesInSet?: number;
  /**
   * The copy this one was declared identical to.
   *
   * The executor re-reads it at execute time: if the keeper is gone by then,
   * quarantining this copy would leave the user with none, so it is skipped and
   * reported instead.
   */
  keeperPath?: string;
}

/** Extension classes the duplicates route filters by. */
export type ExtensionClass = "all" | "images" | "video" | "documents" | "other";

/* -------------------------------------------------------------------------
 * commands.rs — settings
 * ---------------------------------------------------------------------- */

/** One rule pack as the settings page sees it. */
export interface PackView {
  id: string;
  name: string;
  description: string;
  enabled: boolean;
  enabledByDefault: boolean;
  ruleCount: number;
}

/** The cleaner's settings plus the packs that loaded. */
export interface CleanerSettingsView {
  packs: PackView[];
}

/** Duplicate-finder settings. */
export interface DuplicatesSettings {
  minSizeBytes: number;
  roots: string[];
}

/** Quarantine retention, already validated by Rust. */
export interface QuarantineSettings {
  retentionDays: number;
  maxBytes: number;
}

/** Everything the settings page reads and writes. */
export interface AppSettings {
  cleaner: CleanerSettingsView;
  duplicates: DuplicatesSettings;
  quarantine: QuarantineSettings;
}

/* -------------------------------------------------------------------------
 * core/quarantine.rs
 * ---------------------------------------------------------------------- */

/** One row of the quarantine ledger. */
export interface QuarantineItem {
  id: string;
  batchId: string;
  originalPath: string;
  quarantinePath: string;
  sizeBytes: number;
  /** RFC 3339 UTC. */
  quarantinedAt: string;
  purgeAfter: string;
  restoredAt: string | null;
}

/** A batch as the drawer shows it. */
export interface QuarantineBatch {
  batchId: string;
  itemCount: number;
  sizeBytes: number;
  quarantinedAt: string;
  purgeAfter: string;
}

/** Totals across everything still held. */
export interface QuarantineStats {
  itemCount: number;
  totalBytes: number;
  /** Ledger rows that no longer parse and are therefore not counted above. */
  unreadableRows: number;
}

/** What restoring one item did. */
export interface RestoreOutcome {
  itemId: string;
  restoredTo: string;
  /** True when the original path was occupied and a suffix was added. */
  renamed: boolean;
}

/** One item that could not be restored. */
export interface RestoreFailure {
  itemId: string;
  error: string;
}

/** What restoring a batch did. */
export interface RestoreReport {
  restored: RestoreOutcome[];
  failed: RestoreFailure[];
}

/** What a purge removed. */
export interface PurgeReport {
  purgedItems: number;
  freedBytes: number;
  /** Ids whose files could not be removed; the next purge retries them. */
  failedItems: string[];
  /**
   * Batches this purge took items out of. The size cap evicts oldest-first
   * across batch boundaries, so a batch can lose part of itself.
   */
  affectedBatches: string[];
}

/* -------------------------------------------------------------------------
 * modules/shutdown_timer.rs
 * ---------------------------------------------------------------------- */

/** What happens when a timer reaches zero. */
export type PowerAction = "shutdown" | "restart" | "sleep";

/** How the user expressed the target. */
export type ScheduleMode =
  | { mode: "in"; minutes: number }
  | { mode: "at"; hour: number; minute: number };

/** The armed timer. */
export interface ActiveTimerView {
  id: string;
  action: PowerAction;
  /** RFC 3339 UTC. */
  target: string;
  scheduledAt: string;
  remainingSecs: number;
}

/** Current timer state. */
export interface TimerStatus {
  active: ActiveTimerView | null;
  /** True when this build logs power actions instead of performing them. */
  dryRun: boolean;
}
