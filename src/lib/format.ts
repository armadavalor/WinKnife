/**
 * Display formatting shared by every route.
 *
 * Kept out of components so a number reads the same wherever it appears — the
 * summary header, an item row and the quarantine drawer must agree on what
 * "1.2 GB" means, or the app looks like it is guessing.
 */

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

/** Human-readable size. Binary units, because that is what Explorer shows. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) {
    return "0 B";
  }
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < BYTE_UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const decimals = unit === 0 ? 0 : value < 10 ? 1 : 0;
  return `${value.toFixed(decimals)} ${BYTE_UNITS[unit]}`;
}

/** `2h 05m 30s`, matching the tray tooltip in `modules/shutdown_timer.rs`. */
export function formatRemaining(totalSecs: number): string {
  const total = Math.max(0, Math.floor(totalSecs));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");

  if (hours > 0) {
    return `${hours}h ${pad(minutes)}m ${pad(seconds)}s`;
  }
  if (minutes > 0) {
    return `${minutes}m ${pad(seconds)}s`;
  }
  return `${seconds}s`;
}

/**
 * A count with digit grouping (`2,530,292`), in the system locale like
 * {@link formatDateTime}. Only for counts: ids and codes stay ungrouped.
 */
export function formatCount(count: number, locale?: string): string {
  return count.toLocaleString(locale);
}

/** Local date and time, for "will run at" and "kept until". */
export function formatDateTime(rfc3339: string): string {
  const date = new Date(rfc3339);
  if (Number.isNaN(date.getTime())) {
    return rfc3339;
  }
  return date.toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

/** Rough age, e.g. `3 days`. Used for the "kept until" countdown. */
export function formatRelativeDays(rfc3339: string, from = new Date()): string {
  const date = new Date(rfc3339);
  if (Number.isNaN(date.getTime())) {
    return rfc3339;
  }
  const days = Math.round(
    (date.getTime() - from.getTime()) / (24 * 60 * 60 * 1000),
  );
  if (days <= 0) {
    return "today";
  }
  return days === 1 ? "1 day" : `${days} days`;
}

/**
 * Middle-truncate a path so both the drive and the file name stay visible.
 * A path is most useful at its ends; the middle is the part nobody reads.
 */
export function truncatePath(path: string, max = 64): string {
  if (path.length <= max) {
    return path;
  }
  const keep = Math.floor((max - 1) / 2);
  return `${path.slice(0, keep)}…${path.slice(-keep)}`;
}
