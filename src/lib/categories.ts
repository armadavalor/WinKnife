/**
 * Sidebar tabs: each tool's manifest names a category, and the sidebar lists
 * one category at a time.
 */
import type { ModuleCategory } from "$lib/types";

/** Tab order in the sidebar. */
export const CATEGORY_ORDER: readonly ModuleCategory[] = [
  "cleanup",
  "files",
  "system",
  "apps",
  "network",
  "power",
  "other",
];

/** A tool's category; an unknown or missing one falls back to "other". */
export function categoryOf(module: { category?: string }): ModuleCategory {
  const found = CATEGORY_ORDER.find((category) => category === module.category);
  return found ?? "other";
}

export interface CategoryGroup<T> {
  category: ModuleCategory;
  modules: T[];
}

/** Non-empty categories in tab order, each keeping registry order inside. */
export function groupByCategory<T extends { category?: string }>(
  modules: readonly T[],
): CategoryGroup<T>[] {
  return CATEGORY_ORDER.map((category) => ({
    category,
    modules: modules.filter((module) => categoryOf(module) === category),
  })).filter((group) => group.modules.length > 0);
}

/** The tab to show: the wanted one if it still has tools, else the first. */
export function pickCategory(
  groups: readonly CategoryGroup<unknown>[],
  wanted: string | null,
): ModuleCategory | null {
  const match = groups.find((group) => group.category === wanted);
  return (match ?? groups[0])?.category ?? null;
}
