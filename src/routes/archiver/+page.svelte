<!--
  Big-file archiver route: Scan → Review → Move.

  Lists large files unused for a long time. Nothing starts selected. A move
  copies each file, checks the copy, and only then sends the original to
  quarantine, so it can always be undone.
-->
<script lang="ts">
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import RiskBadge from "$lib/components/RiskBadge.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import InfoTip from "$lib/components/InfoTip.svelte";
  import ListToolbar from "$lib/components/ListToolbar.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { pickFolder } from "$lib/api";
  import { formatBytes, formatCount, truncatePath } from "$lib/format";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import {
    deselectVisible,
    hiddenSelectedCount,
    invertVisible,
    matchesFilter,
    selectVisible,
  } from "$lib/listTools";
  import { archiverStore } from "$lib/stores/archiver.svelte";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { quarantineStore } from "$lib/stores/quarantine.svelte";

  const MODULE_ID = "archiver";
  // Shown before the first scan; the scan result carries the values it used.
  const DEFAULT_MIN_BYTES = 1024 ** 3;
  const DEFAULT_MIN_DAYS = 365;

  const manifest = $derived(moduleStore.find(MODULE_ID));
  const store = archiverStore;
  const result = $derived(store.result);

  let confirming = $state(false);
  let filter = $state("");

  async function browse(): Promise<void> {
    const [picked] = await pickFolder();
    if (picked) {
      store.destination = picked;
    }
  }

  async function confirmMove(): Promise<void> {
    confirming = false;
    await store.move();
    await quarantineStore.reload();
  }

  const visibleFindings = $derived(
    (result?.findings ?? []).filter((finding) =>
      matchesFilter(filter, finding.path, store.recommendationFor(finding)?.reason),
    ),
  );
  const pickable = $derived(
    visibleFindings.map((finding) => ({
      id: finding.id,
      selectable: store.isSelectable(store.recommendationFor(finding)),
    })),
  );
  const totalBytes = $derived(
    (result?.findings ?? []).reduce((sum, finding) => sum + finding.sizeBytes, 0),
  );
  const moved = $derived(
    store.report?.items.filter((item) => item.outcome === "archived").length ?? 0,
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
        <span class="text-ink-muted">{t("archiver.rootLabel")}</span>
        <input
          type="text"
          class="rounded-lg border border-surface-3 bg-surface-0 px-3 py-2 font-mono text-xs"
          placeholder="C:\Users\…"
          bind:value={store.root}
        />
        <span class="text-xs text-ink-faint">{t("archiver.rootHint")}</span>
      </label>
      <div class="flex items-center gap-2">
        <button type="button" class="btn-primary" onclick={() => store.scan()}>
          {t("archiver.scan")}
        </button>
        <InfoTip
          text={t("archiver.intro", {
            size: formatBytes(DEFAULT_MIN_BYTES),
            days: DEFAULT_MIN_DAYS,
          })}
        />
      </div>
    </section>
  {:else if store.step === "scanning"}
    <section
      class="flex flex-col gap-3 rounded-xl border border-surface-2 bg-surface-1 p-6"
    >
      <p class="text-sm" role="status">{t("archiver.scanning")}</p>
      <ProgressBar label={t("archiver.scanning")} pct={null} />
      <p class="font-mono text-xs text-ink-faint">
        {store.currentPath ? truncatePath(store.currentPath, 70) : "…"}
      </p>
      <button
        type="button"
        class="self-start btn-ghost px-3! py-1.5!"
        disabled={store.stopping}
        onclick={() => store.stop()}
      >
        {store.stopping ? t("archiver.stopping") : t("archiver.stop")}
      </button>
    </section>
  {:else if store.step === "moving"}
    <section
      class="flex flex-col gap-3 rounded-xl border border-surface-2 bg-surface-1 p-6"
    >
      <p class="text-sm" role="status">{t("archiver.moving")}</p>
      <ProgressBar label={t("archiver.moving")} pct={null} />
    </section>
  {:else if store.step === "done" && store.report}
    <section
      class="flex flex-col items-start gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6"
    >
      <h2 class="text-lg font-semibold tracking-tight">{t("archiver.doneTitle")}</h2>
      <p class="text-sm text-ink-muted">
        {t("archiver.doneBody", { count: moved, manifest: store.report.manifestPath })}
      </p>
      {#each store.report.items as item (item.findingId)}
        {#if item.outcome === "archived"}
          {#if item.manifestWarning}
            <p class="text-xs text-risk-review">
              <span class="font-mono">{truncatePath(item.path, 48)}</span> —
              {item.manifestWarning}
            </p>
          {/if}
        {:else}
          <p class="text-xs text-ink-faint">
            <span class="font-mono">{truncatePath(item.path, 48)}</span> —
            {t(`archiver.${item.outcome}`)}: {item.reason}
          </p>
        {/if}
      {/each}
      <div class="flex gap-2">
        <button type="button" class="btn-primary" onclick={() => quarantineStore.show()}>
          {t("cleaner.openQuarantine")}
        </button>
        <button type="button" class="btn-ghost" onclick={() => store.reset()}>
          {t("archiver.scanAgain")}
        </button>
      </div>
    </section>
  {:else if store.step === "review" && result}
    <section
      class="flex flex-wrap items-center gap-4 rounded-xl border border-surface-2 bg-surface-1 px-5 py-4"
    >
      <p class="flex-1 text-sm leading-relaxed">
        {t("archiver.summary", {
          count: formatCount(result.findings.length),
          size: formatBytes(totalBytes),
          min: formatBytes(result.minSizeBytes),
          days: result.minAgeDays,
        })}
      </p>
      <button
        type="button"
        class="btn-ghost px-3! py-1.5!"
        onclick={() => store.reset()}
      >
        {t("archiver.scanAgain")}
      </button>
    </section>

    {#if result.cancelled}
      <p class="rounded-xl bg-surface-2/60 p-4 text-sm text-ink-muted">
        {t("archiver.cancelled")}
      </p>
    {/if}
    {#if result.unreadable > 0}
      <p class="rounded-xl bg-surface-2/60 p-4 text-sm text-ink-muted">
        {t("archiver.unreadable", { count: result.unreadable })}
      </p>
    {/if}
    {#if result.skippedProtected > 0}
      <p class="rounded-xl bg-surface-2/60 p-4 text-sm text-ink-muted">
        {t("archiver.skippedProtected", { count: result.skippedProtected })}
      </p>
    {/if}

    {#if result.findings.length === 0}
      <EmptyState message={t("archiver.empty")} />
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

      {#if visibleFindings.length === 0}
        <EmptyState message={t("list.noMatch")} />
      {/if}

      <ul
        class="divide-y divide-surface-2 rounded-xl border border-surface-2 bg-surface-1"
      >
        {#each visibleFindings as finding (finding.id)}
          {@const rec = store.recommendationFor(finding)}
          <li class="flex items-start gap-3 px-4 py-3">
            {#if store.isSelectable(rec)}
              <input
                type="checkbox"
                class="mt-1 size-4 shrink-0 accent-[var(--color-accent)]"
                checked={store.selected.has(finding.id)}
                aria-label={finding.path}
                onchange={(event) =>
                  store.toggle(finding.id, event.currentTarget.checked)}
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
                <span class="shrink-0 text-xs tabular-nums text-ink-muted">
                  {formatBytes(finding.sizeBytes)}
                </span>
                {#if rec}
                  <RiskBadge risk={rec.risk} />
                {/if}
              </span>
              <span class="mt-1 block text-[0.7rem] text-ink-faint">
                {t("archiver.age", { days: store.ageOf(finding) })}
              </span>
              {#if rec}
                <span class="mt-1 block text-xs leading-relaxed text-ink-muted">
                  {rec.reason}
                </span>
              {/if}
            </span>
          </li>
        {/each}
      </ul>

      <div
        class="sticky bottom-0 -mx-8 flex flex-wrap items-end gap-4 border-t border-surface-2 bg-surface-0/95 px-8 py-4 backdrop-blur"
      >
        <label class="flex min-w-64 flex-1 flex-col gap-1 text-sm">
          <span class="text-ink-muted">{t("archiver.destLabel")}</span>
          <span class="flex gap-2">
            <input
              type="text"
              class="min-w-0 flex-1 rounded-lg border border-surface-3 bg-surface-0 px-3 py-2 font-mono text-xs"
              placeholder="D:\Archive"
              bind:value={store.destination}
            />
            <button type="button" class="btn-ghost px-3! py-1.5!" onclick={browse}>
              {t("archiver.browse")}
            </button>
          </span>
          <span class="text-xs text-ink-faint">{t("archiver.destHint")}</span>
        </label>
        <p class="text-sm">
          {t("archiver.selection", {
            count: store.selected.size,
            size: formatBytes(store.selectedBytes),
          })}
        </p>
        <button
          type="button"
          class="btn-primary"
          disabled={store.selected.size === 0 || store.destination.trim() === ""}
          onclick={() => (confirming = true)}
        >
          {t("archiver.move")}
        </button>
      </div>
    {/if}
  {/if}
</div>

<ConfirmDialog
  open={confirming}
  title={t("archiver.confirmTitle", { count: store.selected.size })}
  message={t("archiver.confirmBody", { dest: store.destination.trim() })}
  confirmLabel={t("archiver.move")}
  onconfirm={confirmMove}
  oncancel={() => (confirming = false)}
/>
