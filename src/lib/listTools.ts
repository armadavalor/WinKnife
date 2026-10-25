/**
 * Pure helpers behind the shared list toolbar: filter, select-visible, invert,
 * sort, and the persisted collapse state.
 *
 * Kept free of Svelte and `$lib` imports so `node --test` can run it directly.
 * Selection helpers only ever touch rows that are both visible and selectable,
 * so "select all" can never tick a red, protected or permanent-delete row.
 */

export interface ListItem {
  id: string;
  /** False for rows the user may not tick (red, protected, kept copies...). */
  selectable: boolean;
}

export type SortKey = "size" | "name";

/** Case-insensitive substring match against any of the fields; blank matches all. */
export function matchesFilter(
  query: string,
  ...fields: (string | null | undefined)[]
): boolean {
  const needle = query.trim().toLowerCase();
  if (needle === "") return true;
  return fields.some((field) => field?.toLowerCase().includes(needle));
}

/** Adds every visible selectable row to the selection. */
export function selectVisible(
  selected: ReadonlySet<string>,
  visible: readonly ListItem[],
): Set<string> {
  const next = new Set(selected);
  for (const item of visible) if (item.selectable) next.add(item.id);
  return next;
}

/** Selected ids that are not among the visible rows (filtered out of view). */
export function hiddenSelectedCount(
  selected: ReadonlySet<string>,
  visible: readonly ListItem[],
): number {
  const ids = new Set(visible.map((row) => row.id));
  let hidden = 0;
  for (const id of selected) if (!ids.has(id)) hidden += 1;
  return hidden;
}

/** Removes every visible row from the selection; hidden rows keep their state. */
export function deselectVisible(
  selected: ReadonlySet<string>,
  visible: readonly ListItem[],
): Set<string> {
  const next = new Set(selected);
  for (const item of visible) next.delete(item.id);
  return next;
}

/** Flips the selection of every visible selectable row. */
export function invertVisible(
  selected: ReadonlySet<string>,
  visible: readonly ListItem[],
): Set<string> {
  const next = new Set(selected);
  for (const item of visible) {
    if (!item.selectable) continue;
    if (next.has(item.id)) next.delete(item.id);
    else next.add(item.id);
  }
  return next;
}

/** A sorted copy: biggest first for size, A-Z for name. */
export function sortRows<T>(
  rows: readonly T[],
  key: SortKey,
  sizeOf: (row: T) => number,
  nameOf: (row: T) => string,
): T[] {
  const cmp =
    key === "size"
      ? (a: T, b: T) => sizeOf(b) - sizeOf(a)
      : (a: T, b: T) => nameOf(a).localeCompare(nameOf(b));
  return [...rows].sort(cmp);
}

/** Parses a stored collapse list; anything unreadable means "nothing collapsed". */
export function parseCollapsed(raw: string | null): Set<string> {
  try {
    const parsed: unknown = JSON.parse(raw ?? "[]");
    return new Set(
      Array.isArray(parsed)
        ? parsed.filter((id): id is string => typeof id === "string")
        : [],
    );
  } catch {
    return new Set();
  }
}

/** Returns the one description shared by every row, or null when they differ. */
export function sharedText(texts: readonly string[]): string | null {
  const first = texts[0];
  return first !== undefined && texts.every((text) => text === first)
    ? first
    : null;
}
