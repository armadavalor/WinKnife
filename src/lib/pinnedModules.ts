/**
 * Tools the user pinned to the dashboard. Order is the order pinned. A fresh
 * install (nothing stored) gets the starter set; a stored empty list stays
 * empty, so unpinning everything sticks.
 */
export const DEFAULT_PINNED = ["cleaner", "treemap", "duplicates", "startup", "apps", "shutdown-timer"];

/** A stored list, read back; nothing stored means the starter set. */
export function parsePinnedModules(raw: string | null): string[] {
  if (raw === null) return [...DEFAULT_PINNED];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [...DEFAULT_PINNED];
    return [...new Set(parsed.filter((id): id is string => typeof id === "string"))];
  } catch {
    return [...DEFAULT_PINNED];
  }
}

/** The list with `id` appended if absent, removed if present. */
export function togglePinned(pinned: readonly string[], id: string): string[] {
  return pinned.includes(id) ? pinned.filter((other) => other !== id) : [...pinned, id];
}

/** Pinned modules that exist, in pinned order. */
export function pinnedModules<T extends { id: string }>(
  modules: readonly T[],
  pinned: readonly string[],
): T[] {
  return pinned.flatMap((id) => modules.filter((m) => m.id === id));
}
