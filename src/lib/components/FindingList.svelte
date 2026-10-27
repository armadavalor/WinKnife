<!--
  One risk bucket, grouped by the rule pack that produced it.

  Every row carries its `reason` — no unexplained suggestions (CONTRIBUTING.md). It
  is shown as one truncated line (full text in the tooltip, and on click). The
  red bucket has no checkboxes at all: it is informational, and offering a
  disabled checkbox would only look like a bug.

  A full `%TEMP%` is tens of thousands of rows, so each group renders a page
  at a time ("Show more"). The group checkbox and the totals still cover every
  row, shown or not.
-->
<script lang="ts">
  import Icon from "$lib/components/Icon.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import RiskBadge from "$lib/components/RiskBadge.svelte";
  import { formatBytes, truncatePath } from "$lib/format";
  import { t } from "$lib/i18n";
  import { parseCollapsed, sharedText } from "$lib/listTools";
  import {
    findingMeta,
    type Finding,
    type Recommendation,
    type RiskBucket,
  } from "$lib/types";

  interface Row {
    recommendation: Recommendation;
    finding: Finding;
  }

  interface Group {
    packId: string;
    packName: string;
    rows: Row[];
    bytes: number;
    /** The reason every row in the group shares, or null when they differ. */
    reason: string | null;
  }

  interface Props {
    /** Which bucket this list renders. */
    risk: RiskBucket;
    /** Rows already filtered to this bucket. */
    rows: Row[];
    /** Selected recommendation ids. Ignored for the red bucket. */
    selected: Set<string>;
    /** Called when a row is toggled. */
    ontoggle: (recommendationId: string, next: boolean) => void;
    /** Called when a whole group is toggled. */
    ontogglegroup: (recommendationIds: string[], next: boolean) => void;
  }

  let { risk, rows, selected, ontoggle, ontogglegroup }: Props = $props();

  const selectable = $derived(risk !== "doNotDelete");

  /** Rows rendered per group before "Show more". */
  const PAGE_SIZE = 200;

  /** Rows rendered per pack, once somebody asked for more than one page. */
  let shown = $state<Record<string, number>>({});

  function shownIn(packId: string): number {
    return shown[packId] ?? PAGE_SIZE;
  }

  function showMore(packId: string): void {
    shown = { ...shown, [packId]: shownIn(packId) + PAGE_SIZE };
  }

  /**
   * Packs the user has collapsed.
   *
   * Collapsed rather than expanded, so a group only ever hides itself once
   * somebody has asked it to: a scan that found nine hundred temp files opens
   * showing all of them, and the user folds away what they have decided about.
   */
  const STORAGE_KEY = $derived(`winknife.collapsed.cleaner.${risk}`);

  function loadCollapsed(): Set<string> {
    try {
      return parseCollapsed(localStorage.getItem(STORAGE_KEY));
    } catch {
      return new Set();
    }
  }

  let collapsed = $state<Set<string>>(loadCollapsed());

  function toggleCollapsed(packId: string): void {
    const next = new Set(collapsed);
    if (next.has(packId)) {
      next.delete(packId);
    } else {
      next.add(packId);
    }
    collapsed = next;
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify([...next]));
    } catch {
      // Remembering the fold is a convenience; a blocked store just forgets it.
    }
  }

  /** Rows whose reason is shown in full; the others show one truncated line. */
  let expanded = $state<Set<string>>(new Set());

  function toggleExpanded(id: string): void {
    const next = new Set(expanded);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    expanded = next;
  }

  const groups = $derived.by<Group[]>(() => {
    const byPack = new Map<string, Group>();
    for (const row of rows) {
      const meta = findingMeta(row.finding.meta);
      const packId = meta?.packId ?? "other";
      const packName = meta?.packName ?? t("cleaner.otherGroup");
      const group = byPack.get(packId) ?? {
        packId,
        packName,
        rows: [],
        bytes: 0,
        reason: null,
      };
      group.rows.push(row);
      group.bytes += row.recommendation.savingsBytes;
      byPack.set(packId, group);
    }
    const list = [...byPack.values()];
    for (const group of list) {
      // One reason shared by every row is said once, under the header.
      group.reason = sharedText(group.rows.map((r) => r.recommendation.reason));
    }
    return list.sort((a, b) => a.packName.localeCompare(b.packName));
  });

  function groupState(group: Group): "all" | "some" | "none" {
    let chosen = 0;
    for (const row of group.rows) {
      if (selected.has(row.recommendation.id)) chosen += 1;
    }
    if (chosen === 0) return "none";
    return chosen === group.rows.length ? "all" : "some";
  }
</script>

{#if rows.length === 0}
  <EmptyState message={t("cleaner.bucketEmpty")} />
{:else}
  <div class="flex flex-col gap-4">
    {#each groups as group (group.packId)}
      {@const state = groupState(group)}
      {@const open = !collapsed.has(group.packId)}
      {@const limit = shownIn(group.packId)}
      {@const hidden = Math.max(0, group.rows.length - limit)}
      <section class="rounded-xl border border-surface-2 bg-surface-1">
        <header
          class="flex items-center gap-3 border-b border-surface-2 px-4 py-3"
          class:border-b-0={!open}
        >
          {#if selectable}
            <input
              type="checkbox"
              class="size-4 accent-[var(--color-accent)]"
              checked={state === "all"}
              indeterminate={state === "some"}
              aria-label={t("cleaner.selectGroup", { group: group.packName })}
              onchange={(event) =>
                ontogglegroup(
                  group.rows.map((row) => row.recommendation.id),
                  event.currentTarget.checked,
                )}
            />
          {/if}

          <button
            type="button"
            class="flex flex-1 items-center gap-2 text-left text-sm font-medium transition-colors hover:text-ink"
            aria-expanded={open}
            aria-controls="group-{risk}-{group.packId}"
            onclick={() => toggleCollapsed(group.packId)}
          >
            <Icon
              name="chevron-right"
              class="size-4 shrink-0 text-ink-muted transition-transform duration-150"
              style={open ? "transform: rotate(90deg)" : undefined}
            />
            <span>{group.packName}</span>
            <span class="sr-only">
              {open
                ? t("cleaner.collapseGroup", { group: group.packName })
                : t("cleaner.expandGroup", { group: group.packName })}
            </span>
          </button>

          <RiskBadge {risk} count={group.rows.length} />
          {#if group.bytes > 0}
            <span class="text-xs tabular-nums text-ink-muted">
              {formatBytes(group.bytes)}
            </span>
          {/if}
        </header>

        {#if open && group.reason && group.rows.length > 1}
          <p class="border-b border-surface-2 px-4 py-2 text-xs leading-relaxed text-ink-muted">
            {group.reason}
          </p>
        {/if}

        <ul
          id="group-{risk}-{group.packId}"
          class="divide-y divide-surface-2"
          class:hidden={!open}
        >
          {#each group.rows.slice(0, limit) as row (row.recommendation.id)}
            {@const isOpen = expanded.has(row.recommendation.id)}
            <li class="flex items-start gap-3 px-4 py-3">
              {#if selectable}
                <input
                  type="checkbox"
                  class="mt-1 size-4 shrink-0 accent-[var(--color-accent)]"
                  checked={selected.has(row.recommendation.id)}
                  aria-label={row.finding.path}
                  onchange={(event) =>
                    ontoggle(
                      row.recommendation.id,
                      event.currentTarget.checked,
                    )}
                />
              {:else}
                <span class="mt-1 size-4 shrink-0" aria-hidden="true"></span>
              {/if}

              <button
                type="button"
                class="min-w-0 flex-1 text-left"
                aria-expanded={isOpen}
                onclick={() => toggleExpanded(row.recommendation.id)}
              >
                <span
                  class="block truncate font-mono text-xs text-ink"
                  title={row.finding.path}
                >
                  {truncatePath(row.finding.path)}
                </span>
                {#if !group.reason || group.rows.length === 1}
                  <span
                    class="mt-1 block text-xs leading-relaxed text-ink-muted"
                    class:truncate={!isOpen}
                    title={row.recommendation.reason}
                  >
                    {row.recommendation.reason}
                  </span>
                {/if}
              </button>

              <span class="shrink-0 text-xs tabular-nums text-ink-faint">
                {formatBytes(row.finding.sizeBytes)}
              </span>
            </li>
          {/each}
        </ul>
        {#if open && hidden > 0}
          <div class="border-t border-surface-2 px-4 py-3">
            <button
              type="button"
              class="btn-ghost px-3! py-1.5! text-xs"
              onclick={() => showMore(group.packId)}
            >
              {t("cleaner.showMore", {
                count: Math.min(PAGE_SIZE, hidden),
                hidden,
              })}
            </button>
          </div>
        {/if}
      </section>
    {/each}
  </div>
{/if}
