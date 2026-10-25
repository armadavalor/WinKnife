<!--
  Empty-folders route: Scan → Review → Execute, as in module spec 08.

  Only the topmost empty folder of each empty subtree is listed, with a count
  of what is nested inside. Nothing starts selected, and removal is a move to
  quarantine, so it can always be undone.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import ExecutingPanel from "$lib/components/ExecutingPanel.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import RiskBadge from "$lib/components/RiskBadge.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import InfoTip from "$lib/components/InfoTip.svelte";
  import ListToolbar from "$lib/components/ListToolbar.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { formatCount, truncatePath } from "$lib/format";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import {
    deselectVisible,
    hiddenSelectedCount,
    invertVisible,
    matchesFilter,
    selectVisible,
    sharedText,
  } from "$lib/listTools";
  import { emptyFoldersStore } from "$lib/stores/emptyFolders.svelte";
  import { itemsToReport, stopSummary, warningsOf } from "$lib/stores/execNotes";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { quarantineStore } from "$lib/stores/quarantine.svelte";
  import { settingsStore } from "$lib/stores/settings.svelte";

  const MODULE_ID = "empty-folders";

  const manifest = $derived(moduleStore.find(MODULE_ID));
  const store = emptyFoldersStore;
  const result = $derived(store.result);

  let confirming = $state(false);

  const retentionDays = $derived(
    settingsStore.settings?.quarantine.retentionDays ?? 7,
  );

  onMount(() => {
    void settingsStore.load();
  });

  async function confirmExecute(): Promise<void> {
    confirming = false;
    await store.execute();
    await quarantineStore.reload();
  }

  let filter = $state("");

  const visibleFindings = $derived(
    (result?.findings ?? []).filter((finding) =>
      matchesFilter(
        filter,
        finding.path,
        store.recommendationFor(finding)?.reason,
      ),
    ),
  );
  const pickable = $derived(
    visibleFindings.flatMap((finding) => {
      const rec = store.recommendationFor(finding);
      return rec
        ? [{ id: rec.id, selectable: store.isSelectable(rec) }]
        : [];
    }),
  );
  // One reason on every row is said once, above the list.
  const sharedReason = $derived(
    sharedText(
      visibleFindings.map((f) => store.recommendationFor(f)?.reason ?? ""),
    ) || null,
  );

  const succeeded = $derived(
    store.report?.items.filter((item) => item.outcome.status === "succeeded")
      .length ?? 0,
  );
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(manifest.id, manifest.name) : MODULE_ID}
    info={manifest ? tModuleDescription(manifest.id, manifest.description) : undefined}
  />

  {#if store.error}
    <p
      class="rounded-xl border border-risk-danger/30 bg-surface-1 p-4 text-sm text-risk-danger"
    >
      {store.error}
    </p>
  {/if}

  {#if store.step === "idle"}
    <section
      class="flex flex-col items-start gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6"
    >
      <label class="flex w-full max-w-xl flex-col gap-1 text-sm">
        <span class="text-ink-muted">{t("emptyFolders.rootLabel")}</span>
        <input
          type="text"
          class="rounded-lg border border-surface-3 bg-surface-0 px-3 py-2 font-mono text-xs"
          placeholder="C:\Users\…"
          bind:value={store.root}
        />
        <span class="text-xs text-ink-faint">{t("emptyFolders.rootHint")}</span>
      </label>
      <div class="flex items-center gap-2">
        <button
          type="button"
          class="btn-primary"
          onclick={() => store.scan()}
        >
          {t("emptyFolders.scan")}
        </button>
        <InfoTip text={t("emptyFolders.intro")} />
      </div>
    </section>
  {:else if store.step === "scanning"}
    <section
      class="flex flex-col gap-3 rounded-xl border border-surface-2 bg-surface-1 p-6"
    >
      <p class="text-sm">{t("emptyFolders.scanning")}</p>
      <ProgressBar label={t("emptyFolders.scanning")} pct={store.pct} />
      <p class="font-mono text-xs text-ink-faint">
        {store.currentPath ? truncatePath(store.currentPath, 70) : "…"}
      </p>
      <button
        type="button"
        class="self-start btn-ghost px-3! py-1.5!"
        disabled={store.stopping}
        onclick={() => store.stop()}
      >
        {store.stopping ? t("emptyFolders.stopping") : t("emptyFolders.stop")}
      </button>
    </section>
  {:else if store.step === "executing"}
    <ExecutingPanel tracker={store.exec} />
  {:else if store.step === "done" && store.report}
    {@const stopped = stopSummary(store.report.items)}
    <section
      class="flex flex-col items-start gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6"
    >
      <h2 class="text-lg font-semibold tracking-tight">
        {t("emptyFolders.doneTitle")}
      </h2>
      <p class="text-sm text-ink-muted">
        {t("emptyFolders.doneBody", { count: succeeded })}
      </p>
      {#if stopped}
        <p class="text-sm text-risk-review">
          {t("exec.stopped", { moved: stopped.moved, left: stopped.left })}
        </p>
      {/if}
      {#each itemsToReport(store.report.items) as item (item.findingId)}
        {#if item.outcome.status !== "succeeded"}
          <p class="text-xs text-ink-faint">
            <span class="font-mono">{truncatePath(item.path, 48)}</span> —
            {item.outcome.status === "skipped"
              ? item.outcome.reason
              : item.outcome.error}
          </p>
        {/if}
        {#each warningsOf(item) as warning, index (index)}
          <p class="text-xs text-risk-review">
            <span class="font-mono">{truncatePath(warning.path, 48)}</span> —
            {warning.reason}
          </p>
        {/each}
      {/each}
      <div class="flex gap-2">
        <button
          type="button"
          class="btn-primary"
          onclick={() => quarantineStore.show()}
        >
          {t("cleaner.openQuarantine")}
        </button>
        <button
          type="button"
          class="btn-ghost"
          onclick={() => store.reset()}
        >
          {t("emptyFolders.scanAgain")}
        </button>
      </div>
    </section>
  {:else if store.step === "review" && result}
    <section
      class="flex flex-wrap items-center gap-4 rounded-xl border border-surface-2 bg-surface-1 px-5 py-4"
    >
      <p class="flex-1 text-sm leading-relaxed">
        {t("emptyFolders.summary", {
          count: formatCount(result.findings.length),
          visited: formatCount(result.foldersVisited),
        })}
      </p>
      <button
        type="button"
        class="btn-ghost px-3! py-1.5!"
        onclick={() => store.reset()}
      >
        {t("emptyFolders.scanAgain")}
      </button>
    </section>

    {#if result.cancelled}
      <p class="rounded-xl bg-surface-2/60 p-4 text-sm text-ink-muted">
        {t("emptyFolders.cancelled")}
      </p>
    {/if}
    {#if result.unreadable > 0}
      <p class="rounded-xl bg-surface-2/60 p-4 text-sm text-ink-muted">
        {t("emptyFolders.unreadable", { count: result.unreadable })}
      </p>
    {/if}
    {#if result.skippedProtected > 0}
      <p class="rounded-xl bg-surface-2/60 p-4 text-sm text-ink-muted">
        {t("emptyFolders.skippedProtected", { count: result.skippedProtected })}
      </p>
    {/if}

    {#if result.findings.length === 0}
      <EmptyState message={t("emptyFolders.empty")} />
    {:else}
      <ListToolbar
        bind:filter
        shown={visibleFindings.length}
        total={result.findings.length}
        hiddenSelected={filter.trim() ? hiddenSelectedCount(store.selected, pickable) : 0}
        onselectall={() => (store.selected = selectVisible(store.selected, pickable))}
        onselectnone={() => (store.selected = deselectVisible(store.selected, pickable))}
        oninvert={() => (store.selected = invertVisible(store.selected, pickable))}
      />

      {#if sharedReason && visibleFindings.length > 1}
        <p class="text-xs leading-relaxed text-ink-muted">{sharedReason}</p>
      {/if}

      {#if visibleFindings.length === 0}
        <EmptyState message={t("list.noMatch")} />
      {/if}

      <ul
        class="divide-y divide-surface-2 rounded-xl border border-surface-2 bg-surface-1"
      >
        {#each visibleFindings as finding (finding.id)}
          {@const rec = store.recommendationFor(finding)}
          {@const nested = store.nestedOf(finding)}
          <li class="flex items-start gap-3 px-4 py-3">
            {#if store.isSelectable(rec)}
              <input
                type="checkbox"
                class="mt-1 size-4 shrink-0 accent-[var(--color-accent)]"
                checked={store.selected.has(rec.id)}
                aria-label={finding.path}
                onchange={(event) =>
                  store.toggle(rec.id, event.currentTarget.checked)}
              />
            {:else}
              <span class="mt-1 size-4 shrink-0" aria-hidden="true"></span>
            {/if}
            <span class="min-w-0 flex-1">
              <span class="flex items-center gap-2">
                <span
                  class="min-w-0 flex-1 truncate font-mono text-xs text-ink"
                  title={finding.path}
                >
                  {truncatePath(finding.path)}
                </span>
                {#if rec}
                  <RiskBadge risk={rec.risk} />
                {/if}
              </span>
              {#if rec && !(sharedReason && visibleFindings.length > 1)}
                <span class="mt-1 block text-xs leading-relaxed text-ink-muted">
                  {rec.reason}
                </span>
              {/if}
              {#if nested > 0}
                <span class="mt-1 block text-[0.7rem] text-ink-faint">
                  {t("emptyFolders.nested", { count: nested })}
                </span>
              {/if}
            </span>
          </li>
        {/each}
      </ul>

      <div
        class="sticky bottom-0 -mx-8 flex items-center gap-4 border-t border-surface-2 bg-surface-0/95 px-8 py-4 backdrop-blur"
      >
        <p class="flex-1 text-sm">
          {t("emptyFolders.selection", { count: store.selected.size })}
        </p>
        <button
          type="button"
          class="btn-primary"
          disabled={store.selected.size === 0}
          onclick={() => (confirming = true)}
        >
          {t("emptyFolders.remove")}
        </button>
      </div>
    {/if}
  {/if}
</div>

<ConfirmDialog
  open={confirming}
  title={t("emptyFolders.confirmTitle", { count: store.selected.size })}
  message={t("emptyFolders.confirmBody")}
  note={t("cleaner.confirmNote", { days: retentionDays })}
  confirmLabel={t("emptyFolders.remove")}
  onconfirm={confirmExecute}
  oncancel={() => (confirming = false)}
/>
