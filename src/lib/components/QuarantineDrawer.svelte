<!--
  The quarantine drawer: proof that "everything reversible" is real.

  It lives in the shell rather than in the cleaner route, because the promise
  is global — whatever removed something, this is where it comes back from.
-->
<script lang="ts">
  import { formatBytes, formatDateTime, formatRelativeDays, truncatePath } from "$lib/format";
  import { t } from "$lib/i18n";
  import { quarantineStore } from "$lib/stores/quarantine.svelte";

  const store = quarantineStore;
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
        class="rounded-lg px-2 py-1 text-sm text-ink-muted transition-colors hover:bg-surface-2 hover:text-ink"
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
        class="rounded-lg bg-surface-2 px-2.5 py-1 text-xs text-ink-muted transition-colors hover:bg-surface-3 hover:text-ink"
        onclick={() => store.purge()}
      >
        {t("quarantine.purgeNow")}
      </button>
    </div>

    <div class="flex-1 overflow-y-auto px-5 py-4">
      {#if store.status === "loading"}
        <p class="text-sm text-ink-faint">{t("quarantine.loading")}</p>
      {:else if store.status === "error"}
        <p class="text-sm text-risk-danger">{store.error}</p>
      {:else if store.batches.length === 0}
        <p
          class="rounded-xl border border-dashed border-surface-3 p-5 text-sm text-ink-faint"
        >
          {t("quarantine.empty")}
        </p>
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
                    class="rounded-lg bg-accent-soft px-2.5 py-1 text-xs font-medium text-ink transition-colors hover:bg-accent hover:text-surface-0"
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
          {#each store.lastRestore.failed as failure (failure.itemId)}
            <p class="mt-1 text-xs text-risk-danger">{failure.error}</p>
          {/each}
        </div>
      {/if}
    </div>
  </aside>
{/if}
