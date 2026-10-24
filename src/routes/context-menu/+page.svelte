<!--
  Context-menu cleaner (C2): every right-click entry, who owns it, and a
  reversible Disable for the ones under your own account.

  Machine-wide (HKLM) entries are listed but never written; their row says so.
-->
<script lang="ts">
  import LoadingNote from "$lib/components/LoadingNote.svelte";
  import { onMount } from "svelte";

  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { contextMenuStore } from "$lib/stores/context-menu.svelte";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import type { ContextMenuEntry } from "$lib/types";

  const MODULE_ID = "context-menu";
  const manifest = $derived(moduleStore.find(MODULE_ID));
  const store = contextMenuStore;

  let pending = $state<ContextMenuEntry | null>(null);

  onMount(() => {
    void store.load();
  });

  function title(entry: ContextMenuEntry): string {
    return entry.label ?? entry.name;
  }

  async function confirmDisable(): Promise<void> {
    const entry = pending;
    pending = null;
    if (entry) {
      await store.disable(entry);
    }
  }
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(manifest.id, manifest.name) : MODULE_ID}
    info={[manifest ? tModuleDescription(manifest.id, manifest.description) : "", t("contextMenu.intro")]
      .filter(Boolean)
      .join(" ")}
  />

  {#if store.error}
    <p class="rounded-xl border border-risk-danger/30 bg-surface-1 p-4 text-sm text-risk-danger">
      {store.error}
    </p>
  {/if}

  {#if store.status === "loading" || store.status === "idle"}
    <LoadingNote text={t("contextMenu.loading")} />
  {:else if store.status === "error"}
    <div class="flex flex-col items-start gap-3 rounded-xl border border-surface-3 bg-surface-1 p-5">
      <p class="text-sm">{t("contextMenu.loadError")}</p>
      <button
        type="button"
        class="btn-secondary px-3! py-1.5!"
        disabled={store.busy !== null}
        onclick={() => store.reload()}
      >
        {t("dashboard.retry")}
      </button>
    </div>
  {:else if store.entries.length === 0}
    <EmptyState message={t("contextMenu.empty")} />
  {:else}
    <div>
      <button
        type="button"
        class="btn-outline px-2.5! py-1! text-xs!"
        disabled={store.busy !== null}
        onclick={() => store.reload()}
      >
        {t("contextMenu.refresh")}
      </button>
    </div>

    <ul class="flex flex-col gap-2">
      {#each store.entries as entry (entry.id)}
        {@const recommendation = store.recommendationOf(entry)}
        <li class="flex flex-wrap items-start gap-4 rounded-xl border border-surface-2 bg-surface-1 p-4">
          <div class="min-w-0 flex-1">
            <p class="flex flex-wrap items-center gap-2">
              <span class="font-medium">{title(entry)}</span>
              <span class="rounded-md bg-surface-2 px-1.5 py-0.5 text-[0.65rem] text-ink-faint">
                {entry.scope}
              </span>
              {#if !entry.editable}
                <span class="rounded-md bg-surface-2 px-1.5 py-0.5 text-[0.65rem] text-ink-faint">
                  {t("contextMenu.machineWide")}
                </span>
              {/if}
              {#if entry.disabled}
                <span class="rounded-md bg-surface-2 px-1.5 py-0.5 text-[0.65rem] text-ink-faint">
                  {t("contextMenu.disabled")}
                </span>
              {/if}
            </p>
            <p class="mt-1 text-xs text-ink-muted">
              {#if entry.publisher}
                {entry.publisher}
              {:else}
                <span class="text-risk-warning">{t("contextMenu.unknownOwner")}</span>
              {/if}
              {#if entry.file}
                · <span class="select-text break-all font-mono text-[0.7rem]">{entry.file}</span>
              {/if}
            </p>
            <p class="mt-1 select-text break-all font-mono text-[0.7rem] text-ink-faint">
              {entry.path}
            </p>
            <details class="group mt-1.5">
              <summary class="cursor-pointer text-xs text-ink-faint hover:text-ink">
                {t("contextMenu.why")}
              </summary>
              <p class="mt-1 text-xs leading-relaxed text-ink-muted">{entry.reason}</p>
              {#if entry.target}
                <p class="mt-1 select-text break-all font-mono text-[0.7rem] text-ink-faint">
                  {entry.target}
                </p>
              {/if}
            </details>
          </div>

          <div class="flex shrink-0 flex-col items-end gap-1.5">
            {#if entry.undoable}
              <button
                type="button"
                class="btn-secondary px-3! py-1! text-xs!"
                disabled={store.busy !== null}
                onclick={() => store.enable(entry, t("contextMenu.reenabled"))}
              >
                {store.busy === entry.id ? t("contextMenu.working") : t("contextMenu.enable")}
              </button>
            {:else if recommendation}
              <button
                type="button"
                class="btn-primary px-3! py-1! text-xs!"
                disabled={store.busy !== null}
                onclick={() => (pending = entry)}
              >
                {store.busy === entry.id ? t("contextMenu.working") : t("contextMenu.disable")}
              </button>
            {:else if !entry.editable}
              <span class="text-[0.7rem] text-ink-muted">{t("contextMenu.needsAdmin")}</span>
            {/if}
          </div>
        </li>
      {/each}
    </ul>

    {#if store.lastReport}
      <p class="rounded-xl bg-surface-2/60 p-4 text-sm text-ink-muted">
        {store.lastReport}
      </p>
    {/if}
  {/if}
</div>

<ConfirmDialog
  open={pending !== null}
  title={t("contextMenu.confirmTitle", { name: pending ? title(pending) : "" })}
  message={t("contextMenu.confirmBody")}
  note={t("contextMenu.confirmNote")}
  detail={pending?.path}
  confirmLabel={t("contextMenu.confirmAction")}
  onconfirm={confirmDisable}
  oncancel={() => (pending = null)}
/>
