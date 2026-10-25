<!--
  The one toolbar every long list shares: text filter, optional sort, and
  select all / none / invert over the rows currently shown.

  It owns no selection logic. The page passes callbacks built on
  `$lib/listTools`, which only ever touch visible, selectable rows.
-->
<script lang="ts">
  import { t } from "$lib/i18n";
  import type { SortKey } from "$lib/listTools";

  interface Props {
    filter: string;
    /** Omit to hide the sort control. */
    sort?: SortKey;
    /** Rows after filtering, and rows before. Hides the counter when equal. */
    shown: number;
    total: number;
    /** Ticked rows the filter hides; they are still acted on, so say so. */
    hiddenSelected?: number;
    /** Omit all three to hide the select buttons (lists with no selection). */
    onselectall?: () => void;
    onselectnone?: () => void;
    oninvert?: () => void;
  }

  let {
    filter = $bindable(),
    sort = $bindable(),
    shown,
    total,
    hiddenSelected = 0,
    onselectall,
    onselectnone,
    oninvert,
  }: Props = $props();
</script>

<div class="flex flex-wrap items-center gap-2">
  <input
    type="search"
    class="min-w-48 flex-1 rounded-lg border border-surface-3 bg-surface-0 px-3 py-1.5 text-sm"
    placeholder={t("list.filter")}
    aria-label={t("list.filter")}
    bind:value={filter}
  />
  {#if sort !== undefined}
    <select
      class="rounded-lg border border-surface-3 bg-surface-0 px-2 py-1.5 text-sm"
      aria-label={t("list.sort")}
      bind:value={sort}
    >
      <option value="size">{t("list.sortSize")}</option>
      <option value="name">{t("list.sortName")}</option>
    </select>
  {/if}
  {#if onselectall}
    <button type="button" class="btn-ghost px-3! py-1.5! text-xs" onclick={onselectall}>
      {t("list.selectAll")}
    </button>
  {/if}
  {#if onselectnone}
    <button type="button" class="btn-ghost px-3! py-1.5! text-xs" onclick={onselectnone}>
      {t("list.selectNone")}
    </button>
  {/if}
  {#if oninvert}
    <button type="button" class="btn-ghost px-3! py-1.5! text-xs" onclick={oninvert}>
      {t("list.invert")}
    </button>
  {/if}
  {#if hiddenSelected > 0}
    <span class="text-xs font-medium text-risk-review" role="status">
      {t("list.hiddenSelected", { count: hiddenSelected })}
    </span>
  {/if}
  {#if shown !== total}
    <span class="text-xs tabular-nums text-ink-faint">
      {t("list.shown", { shown, total })}
    </span>
  {/if}
</div>
