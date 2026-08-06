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
