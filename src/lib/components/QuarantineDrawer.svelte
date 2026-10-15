<!--
  The quarantine drawer: proof that "everything reversible" is real.

  It lives in the shell rather than in the cleaner route, because the promise
  is global — whatever removed something, this is where it comes back from.
-->
<script lang="ts">
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import { formatBytes, formatDateTime, formatRelativeDays, truncatePath } from "$lib/format";
  import { t } from "$lib/i18n";
  import { isRestoreRefused, refusedHeld, warningsOf } from "$lib/stores/execNotes";
  import { quarantineStore } from "$lib/stores/quarantine.svelte";
  import type { QuarantineItem } from "$lib/types";

  const store = quarantineStore;

  /** The item the user asked to remove, waiting for confirmation. */
  let pendingRemoval = $state<QuarantineItem | null>(null);

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
        onclick={() => store.purge()}
      >
        {t("quarantine.purgeNow")}
      </button>
    </div>

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

    {#if store.lastPurge && store.lastPurge.affectedBatches.length > 0}
      <p
        class="border-b border-surface-2 bg-surface-2/60 px-5 py-3 text-xs leading-relaxed text-ink-muted"
      >
        {t("quarantine.purged", {
          count: store.lastPurge.purgedItems,
          size: formatBytes(store.lastPurge.freedBytes),
        })}
        {t("quarantine.affectedBatches", {
          count: store.lastPurge.affectedBatches.length,
        })}
        {#if store.lastPurge.failedItems.length > 0}
          {t("quarantine.purgeFailed", {
            count: store.lastPurge.failedItems.length,
          })}
        {/if}
      </p>
    {/if}

    <div class="flex-1 overflow-y-auto px-5 py-4">
      {#if store.status === "loading"}
        <p class="text-sm text-ink-faint">{t("quarantine.loading")}</p>
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
                  <button
                    type="button"
                    class="btn-primary px-2.5! py-1! text-xs!"
                    onclick={() => store.restore(batch.batchId)}
                  >
                    {t("quarantine.restore")}
                  </button>
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

              {#if store.items[batch.batchId]}
                <ul class="border-t border-surface-2 px-4 py-2">
                  {#each store.items[batch.batchId] as item (item.id)}
                    <li class="py-1.5">
                      <p
                        class="truncate font-mono text-[0.7rem] text-ink-muted"
                        title={item.originalPath}
                      >
                        {truncatePath(item.originalPath, 52)}
                      </p>
                      <p class="text-[0.7rem] text-ink-faint">
                        {formatBytes(item.sizeBytes)}
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
                    </li>
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
            {t("quarantine.restored", {
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
