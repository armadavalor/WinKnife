/**
 * Tools the user hid from the sidebar (issue #8). Hiding is visual only: the
 * dashboard still lists every tool, and its route and `--module <id>` still
 * open it, so a hidden tool is never out of reach.
 */

/** A stored list, read back; anything that is not a list of ids means none. */
export function parseHiddenModules(raw: string | null): string[] {
  if (raw === null) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return [...new Set(parsed.filter((id): id is string => typeof id === "string"))];
  } catch {
    return [];
  }
}

/** The list with `id` hidden if it was shown, shown if it was hidden. */
export function toggleHidden(hidden: readonly string[], id: string): string[] {
  return hidden.includes(id) ? hidden.filter((other) => other !== id) : [...hidden, id];
}

/** The modules still shown, in registry order. */
export function visibleModules<T extends { id: string }>(
  modules: readonly T[],
  hidden: readonly string[],
): T[] {
  return modules.filter((module) => !hidden.includes(module.id));
}
