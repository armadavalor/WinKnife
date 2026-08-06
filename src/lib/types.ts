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
