<!--
  One risk bucket, grouped by the rule pack that produced it.

  Every row carries its `reason` — no unexplained suggestions (CLAUDE.md). It
  is shown as one truncated line (full text in the tooltip, and on click). The
  red bucket has no checkboxes at all: it is informational, and offering a
  disabled checkbox would only look like a bug.
-->
<script lang="ts">
  import EmptyState from "$lib/components/EmptyState.svelte";
  import RiskBadge from "$lib/components/RiskBadge.svelte";
  import { formatBytes, truncatePath } from "$lib/format";
  import { t } from "$lib/i18n";
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

  /**
   * Packs the user has collapsed.
   *
   * Collapsed rather than expanded, so a group only ever hides itself once
   * somebody has asked it to: a scan that found nine hundred temp files opens
   * showing all of them, and the user folds away what they have decided about.
   */
  let collapsed = $state<Set<string>>(new Set());

  function toggleCollapsed(packId: string): void {
    const next = new Set(collapsed);
    if (next.has(packId)) {
      next.delete(packId);
    } else {
      next.add(packId);
    }
    collapsed = next;
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
      };
      group.rows.push(row);
      group.bytes += row.recommendation.savingsBytes;
      byPack.set(packId, group);
    }
    return [...byPack.values()].sort((a, b) =>
      a.packName.localeCompare(b.packName),
    );
  });

  function groupState(group: Group): "all" | "some" | "none" {
    const chosen = group.rows.filter((row) =>
      selected.has(row.recommendation.id),
    ).length;
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
            <svg
              class="size-3.5 shrink-0 text-ink-faint transition-transform"
              style:transform={open ? "rotate(90deg)" : "none"}
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M9 6l6 6-6 6" />
            </svg>
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

        <ul
          id="group-{risk}-{group.packId}"
          class="divide-y divide-surface-2"
          class:hidden={!open}
        >
          {#each group.rows as row (row.recommendation.id)}
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
                <span
                  class="mt-1 block text-xs leading-relaxed text-ink-muted"
                  class:truncate={!isOpen}
                  title={row.recommendation.reason}
                >
                  {row.recommendation.reason}
                </span>
              </button>

              <span class="shrink-0 text-xs tabular-nums text-ink-faint">
                {formatBytes(row.finding.sizeBytes)}
              </span>
            </li>
          {/each}
        </ul>
      </section>
    {/each}
  </div>
{/if}
