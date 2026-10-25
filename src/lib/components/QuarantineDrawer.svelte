<!--
  The quarantine drawer: proof that "everything reversible" is real.

  It lives in the shell rather than in the cleaner route, because the promise
  is global — whatever removed something, this is where it comes back from.
-->
<script lang="ts">
  import LoadingNote from "$lib/components/LoadingNote.svelte";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import { formatBytes, formatDateTime, formatRelativeDays, truncatePath } from "$lib/format";
  import { t } from "$lib/i18n";
  import { isRestoreRefused, refusedHeld, warningsOf } from "$lib/stores/execNotes";
  import { quarantineStore } from "$lib/stores/quarantine.svelte";
  import { filterItems } from "$lib/stores/quarantineView";
  import type { QuarantineItem } from "$lib/types";

  const store = quarantineStore;

  /** The item the user asked to remove, waiting for confirmation. */
  let pendingRemoval = $state<QuarantineItem | null>(null);
  /** "Remove for good now" was pressed and waits for confirmation. */
  let confirmPurge = $state(false);
  /** Path filter for the expanded item lists. */
  let query = $state("");

  const anyExpanded = $derived(Object.keys(store.items).length > 0);
  /** Ids of the items the filter leaves showing, across expanded batches. */
  const shownIds = $derived(
    Object.values(store.items).flatMap((list) => filterItems(list, query).map((item) => item.id)),
  );
  const allShownTicked = $derived(
    shownIds.length > 0 && shownIds.every((id) => store.selected.includes(id)),
  );
  const busy = $derived(store.purging || store.restoringSelected || store.restoring !== null);

  async function runPurge(): Promise<void> {
    confirmPurge = false;
    await store.purge();
  }

  async function confirmRemoval(): Promise<void> {
    const item = pendingRemoval;
    pendingRemoval = null;
    if (item) {
      await store.purgeItem(item);
    }
  }
</script>

{#if store.open}
  <aside
    class="fixed inset-y-0 right-0 z-30 flex w-full max-w-md flex-col border-l border-surface-2 bg-surface-1 shadow-2xl"
    aria-label={t("quarantine.title")}
  >
    <header class="flex items-start gap-3 border-b border-surface-2 px-5 py-4">
      <div class="flex-1">
        <h2 class="text-base font-semibold tracking-tight">
          {t("quarantine.title")}
        </h2>
        <p class="mt-1 text-xs leading-relaxed text-ink-muted">
          {t("quarantine.subtitle")}
        </p>
      </div>
      <button
        type="button"
        class="btn-ghost px-2! py-1!"
        onclick={() => store.hide()}
      >
        {t("action.close")}
      </button>
    </header>

    <div class="flex items-center gap-3 border-b border-surface-2 px-5 py-3">
      <span class="flex-1 text-xs text-ink-muted">
        {t("quarantine.held", {
          count: store.stats.itemCount,
          size: formatBytes(store.stats.totalBytes),
        })}
      </span>
      <button
        type="button"
        class="btn-secondary px-2.5! py-1! text-xs!"
        disabled={busy}
        aria-busy={store.purging}
        onclick={() => (confirmPurge = true)}
      >
        {store.purging ? t("quarantine.purging") : t("quarantine.purgeNow")}
      </button>
    </div>

    {#if anyExpanded}
      <div class="flex flex-col gap-2 border-b border-surface-2 px-5 py-3">
        <input
          type="search"
          class="w-full rounded-lg border border-surface-2 bg-surface-0 px-2.5 py-1 text-xs"
          placeholder={t("quarantine.filter")}
          aria-label={t("quarantine.filter")}
          bind:value={query}
        />
        <div class="flex items-center gap-2 text-xs">
          <label class="flex flex-1 items-center gap-2 text-ink-muted">
            <input
              type="checkbox"
              checked={allShownTicked}
              disabled={shownIds.length === 0}
              onchange={() => store.toggleAll(shownIds)}
            />
            {t("quarantine.selectAll")}
          </label>
          {#if store.selected.length > 0}
            <button
              type="button"
              class="btn-ghost px-2! py-1! text-xs!"
              onclick={() => (store.selected = [])}
            >
              {t("quarantine.clearSelection")}
            </button>
            <button
              type="button"
              class="btn-primary px-2.5! py-1! text-xs!"
              disabled={busy}
              aria-busy={store.restoringSelected}
              onclick={() => store.restoreSelected()}
            >
              {store.restoringSelected
                ? t("quarantine.restoring")
                : t("quarantine.restoreSelected", { count: store.selected.length })}
            </button>
          {/if}
        </div>
      </div>
    {/if}

    <!--
      Architect addition B: two things the ledger has always known and never
      said. A row that no longer parses is not counted in the totals above, so
      the drawer has to admit it exists or the numbers quietly under-report; and
      the size cap evicts oldest-first across batch boundaries, so a batch can
      lose part of itself. Factual tone, no alarm — these are notes, not errors.
    -->
    {#if store.stats.unreadableRows > 0}
      <details class="group border-b border-surface-2 bg-surface-2/60 px-5 py-3">
        <summary class="cursor-pointer text-xs text-ink-faint hover:text-ink">
          {t("quarantine.detailsSummary")}
        </summary>
        <p class="mt-2 text-xs leading-relaxed text-ink-muted">
          {t("quarantine.unreadableRows", { count: store.stats.unreadableRows })}
        </p>
      </details>
    {/if}

    {#if refusedHeld(store.stats)}
      <p
        class="border-b border-surface-2 bg-surface-2/60 px-5 py-3 text-xs leading-relaxed text-ink-muted"
      >
        {t("quarantine.refusedHeld", {
          count: store.stats.refusedItems ?? 0,
          used: formatBytes(store.stats.refusedBytes ?? 0),
          cap: formatBytes(store.stats.capBytes ?? 0),
        })}
      </p>
    {/if}

    {#if store.removeFailed}
      <p
        class="select-text break-words border-b border-surface-2 bg-surface-2/60 px-5 py-3 text-xs leading-relaxed text-risk-danger"
      >
        {#if store.removeFailed.failure.kind === "stuck"}
          {t("quarantine.removeFailed", { path: store.removeFailed.path })}
        {:else if store.removeFailed.failure.kind === "notEligible"}
          {t("quarantine.removeRefused", { path: store.removeFailed.path })}
        {:else if store.removeFailed.failure.kind === "notMarked"}
          {t("quarantine.removeNotMarked", { path: store.removeFailed.path })}
        {:else}
          {t("quarantine.removeError", {
            path: store.removeFailed.path,
            error: store.removeFailed.failure.error,
          })}
        {/if}
      </p>
    {/if}

    {#if store.actionError}
      <p
        role="alert"
        class="select-text break-words border-b border-surface-2 bg-surface-2/60 px-5 py-3 text-xs leading-relaxed text-risk-danger"
      >
        {store.actionError}
      </p>
    {/if}

    {#if store.purgeResult}
      {@const result = store.purgeResult}
      <div
        role="status"
        class="border-b border-surface-2 bg-surface-2/60 px-5 py-3 text-xs leading-relaxed text-ink-muted"
      >
        {#if result.kind === "nothingDue"}
          {result.nextDue
            ? t("quarantine.purgeNothingDue", { when: formatDateTime(result.nextDue) })
            : t("quarantine.purgeNothingHeld")}
        {:else}
          {#if result.count > 0}
            <p class="text-ink">
              {t("quarantine.purged", { count: result.count, size: formatBytes(result.bytes) })}
              {t("quarantine.affectedBatches", { count: result.batches })}
            </p>
          {/if}
          {#if result.failures.length > 0}
            <p class="mt-1 text-risk-danger">
              {t("quarantine.purgeFailed", {
                count: result.failures.reduce((sum, f) => sum + f.count, 0),
              })}
            </p>
            {#each result.failures as failure (failure.reason)}
              <p class="mt-0.5 text-risk-danger">
                {t(`quarantine.purgeReason.${failure.reason}`, { count: failure.count })}
              </p>
            {/each}
          {/if}
        {/if}
      </div>
    {/if}

    <div class="flex-1 overflow-y-auto px-5 py-4">
      {#if store.status === "loading"}
        <LoadingNote text={t("quarantine.loading")} />
      {:else if store.status === "error"}
        <p class="text-sm text-risk-danger">{store.error}</p>
      {:else if store.batches.length === 0}
        <EmptyState message={t("quarantine.empty")} />
      {:else}
        <ul class="flex flex-col gap-3">
          {#each store.batches as batch (batch.batchId)}
            <li class="rounded-xl border border-surface-2 bg-surface-0">
              <div class="flex items-start gap-3 px-4 py-3">
                <div class="min-w-0 flex-1">
                  <p class="text-sm font-medium">
                    {t("quarantine.batchLabel", { count: batch.itemCount })}
                  </p>
                  <p class="mt-0.5 text-xs text-ink-muted">
                    {formatDateTime(batch.quarantinedAt)} ·
                    {formatBytes(batch.sizeBytes)}
                  </p>
                  <p class="mt-0.5 text-xs text-ink-faint">
                    {t("quarantine.keptFor", {
                      time: formatRelativeDays(batch.purgeAfter),
                    })}
                  </p>
                </div>
                <div class="flex shrink-0 flex-col gap-1">
                  {#if store.restoring?.batchId === batch.batchId}
                    <button
                      type="button"
                      class="btn-ghost px-2.5! py-1! text-xs!"
                      disabled={store.restoring.stopping}
                      onclick={() => store.stopRestore()}
                    >
                      {store.restoring.stopping ? t("cleaner.stopping") : t("cleaner.stop")}
                    </button>
                  {:else}
                    <button
                      type="button"
                      class="btn-primary px-2.5! py-1! text-xs!"
                      disabled={store.restoring !== null}
                      onclick={() => store.restore(batch.batchId)}
                    >
                      {t("quarantine.restore")}
                    </button>
                  {/if}
                  <button
                    type="button"
                    class="rounded-lg px-2.5 py-1 text-xs text-ink-muted transition-colors hover:bg-surface-2 hover:text-ink"
                    onclick={() => store.toggleBatch(batch.batchId)}
                  >
                    {store.items[batch.batchId]
                      ? t("quarantine.hideItems")
                      : t("quarantine.showItems")}
                  </button>
                </div>
              </div>
              {#if store.restoring?.batchId === batch.batchId}
                {@const run = store.restoring}
                <div class="flex flex-col gap-1.5 px-4 pb-3">
                  <ProgressBar
                    label={t("quarantine.restoring")}
                    pct={run.total ? Math.floor((run.done / run.total) * 100) : null}
                  />
                  <p class="font-mono text-xs text-ink-faint">
                    {run.currentPath ? truncatePath(run.currentPath, 48) : "…"}
                  </p>
                  {#if run.total !== null}
                    <p class="text-xs text-ink-faint">
                      {t("exec.progress", { done: run.done, total: run.total })}
                    </p>
                  {/if}
                </div>
              {/if}

              {#if store.items[batch.batchId]}
                <ul class="border-t border-surface-2 px-4 py-2">
                  {#each filterItems(store.items[batch.batchId], query) as item (item.id)}
                    {@const failedWhy = store.lastPurge?.failureReasons?.[item.id]}
                    <li class="flex gap-2 py-1.5">
                      <input
                        type="checkbox"
                        class="mt-0.5 shrink-0"
                        aria-label={t("quarantine.selectItem", { path: item.originalPath })}
                        checked={store.selected.includes(item.id)}
                        onchange={() => store.toggleSelected(item.id)}
                      />
                      <div class="min-w-0 flex-1">
                      <p
                        class="truncate font-mono text-[0.7rem] text-ink-muted"
                        title={item.originalPath}
                      >
                        {truncatePath(item.originalPath, 52)}
                      </p>
                      <p class="text-[0.7rem] text-ink-faint">
                        {formatBytes(item.sizeBytes)}
                        {#if failedWhy}
                          <span class="ml-1 rounded bg-risk-danger/15 px-1.5 py-px text-risk-danger">
                            {t(`quarantine.failedBadge.${failedWhy}`)}
                          </span>
                        {/if}
                      </p>
                      {#if isRestoreRefused(item)}
                        <div class="mt-1 flex items-center gap-2">
                          <p class="flex-1 text-[0.7rem] text-risk-review">
                            {t("quarantine.restoreRefused")}
                          </p>
                          <button
                            type="button"
                            class="rounded-lg px-2 py-0.5 text-[0.7rem] text-risk-danger transition-colors hover:bg-surface-2"
                            onclick={() => (pendingRemoval = item)}
                          >
                            {t("quarantine.removeItem")}
                          </button>
                        </div>
                        <p
                          class="mt-0.5 select-text break-all font-mono text-[0.7rem] text-ink-faint"
                        >
                          {t("quarantine.heldAt")}: {item.quarantinePath}
                        </p>
                      {/if}
                      </div>
                    </li>
                  {:else}
                    <li class="py-1.5 text-[0.7rem] text-ink-faint">{t("quarantine.noMatch")}</li>
                  {/each}
                </ul>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}

      {#if store.lastRestore}
        <div class="mt-4 rounded-xl border border-surface-2 bg-surface-0 p-4">
          <p class="text-sm">
            {store.lastRestore.cancelled
              ? t("quarantine.restoreStopped", {
                  count: store.lastRestore.restored.length,
                })
              : t("quarantine.restored", {
                  count: store.lastRestore.restored.length,
                })}
          </p>
          {#each store.lastRestore.restored.filter((r) => r.renamed) as renamed (renamed.itemId)}
            <p class="mt-1 text-xs text-ink-muted">
              {t("quarantine.restoredAlongside", {
                path: truncatePath(renamed.restoredTo, 48),
              })}
            </p>
          {/each}
          {#each store.lastRestore.restored as outcome (outcome.itemId)}
            {#each warningsOf(outcome) as warning, index (index)}
              <p class="mt-1 text-xs text-risk-review">
                <span class="font-mono">{truncatePath(warning.path, 48)}</span> —
                {warning.reason}
              </p>
            {/each}
          {/each}
          {#each store.lastRestore.failed as failure (failure.itemId)}
            <p class="mt-1 text-xs text-risk-danger">{failure.error}</p>
          {/each}
        </div>
      {/if}
    </div>
  </aside>
{/if}

<ConfirmDialog
  open={confirmPurge}
  title={t("quarantine.purgeTitle")}
  message={t("quarantine.purgeBody")}
  warning={t("quarantine.purgeWarning")}
  confirmLabel={t("quarantine.purgeNow")}
  onconfirm={runPurge}
  oncancel={() => (confirmPurge = false)}
/>

<ConfirmDialog
  open={pendingRemoval !== null}
  title={t("quarantine.removeTitle")}
  message={t("quarantine.removeBody", {
    path: truncatePath(pendingRemoval?.originalPath ?? "", 48),
  })}
  note={t("quarantine.removeNote")}
  detail={pendingRemoval?.quarantinePath}
  warning={t("quarantine.removeWarning")}
  confirmLabel={t("quarantine.removeAction")}
  onconfirm={confirmRemoval}
  oncancel={() => (pendingRemoval = null)}
/>
