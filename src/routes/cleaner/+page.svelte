<!--
  Cleaner route: the 3-step pattern from ARCHITECTURE section 7.

  Scan → Review (three buckets, every item shows its reason) → Execute (a
  confirm dialog that states the total and where the files go). The red bucket
  renders with no action button at all — that is the honesty feature, not an
  oversight.
-->
<script lang="ts">
  import { page } from "$app/state";
  import { onMount } from "svelte";

  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import FindingList from "$lib/components/FindingList.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import RiskBadge from "$lib/components/RiskBadge.svelte";
  import InfoTip from "$lib/components/InfoTip.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import {
    formatBytes,
    formatCount,
    formatDateTime,
    truncatePath,
  } from "$lib/format";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { dashboardStore } from "$lib/stores/dashboard.svelte";
  import { itemsToReport, warningsOf } from "$lib/stores/execNotes";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { quarantineStore } from "$lib/stores/quarantine.svelte";
  import { settingsStore } from "$lib/stores/settings.svelte";
  import { scanStore } from "$lib/stores/scan.svelte";

  const MODULE_ID = "cleaner";

  const manifest = $derived(moduleStore.find(MODULE_ID));
  const store = scanStore;
  const lastScan = $derived(dashboardStore.lastScan(MODULE_ID));

  let confirming = $state(false);

  // The dialog states how long a restore stays possible, and that window is a
  // setting. Reading the stored value rather than writing "7 days" into the
  // sentence keeps the promise true for somebody who changed it.
  const retentionDays = $derived(
    settingsStore.settings?.quarantine.retentionDays ?? 7,
  );

  // Run-once mounting, deliberately not reactive: as an effect this subscribed
  // to the settings store's `status` and reloaded on every transition.
  onMount(() => {
    void settingsStore.load();
    void dashboardStore.load();
  });

  // The treemap's "Scan with Cleaner" cross-link arrives with `?from=`. The
  // cleaner does not scan a folder you pick — it scans the locations its rule
  // packs name — so instead of pretending the path was applied, the route says
  // plainly what it is about to do (module spec 02, and honesty over glue).
  const cameFrom = $derived(page.url.searchParams.get("from"));

  const safeRows = $derived(store.rowsIn("safe"));
  const reviewRows = $derived(store.rowsIn("needsReview"));
  const blockedRows = $derived(store.rowsIn("doNotDelete"));

  // The dialog names emptying the recycle bin separately, in the danger
  // colour, because it is the one thing here that quarantine cannot undo. The
  // flag Rust receives is that warning being on screen when the user confirmed
  // — not merely a recycle-bin row being ticked (spec 02, security review H6).
  const emptiesRecycleBin = $derived(store.selectionEmptiesRecycleBin);

  function resetScan(): void {
    store.reset();
    void dashboardStore.load();
  }

  async function confirmExecute(): Promise<void> {
    const confirmedRecycleBin = emptiesRecycleBin;
    confirming = false;
    await store.execute(confirmedRecycleBin);
    await quarantineStore.reload();
  }
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(manifest.id, manifest.name) : MODULE_ID}
    info={manifest ? tModuleDescription(manifest.id, manifest.description) : undefined}
  />

  {#if cameFrom}
    <p class="rounded-xl bg-surface-2 p-4 text-sm leading-relaxed text-ink-muted">
      {t("cleaner.cameFrom", { path: truncatePath(cameFrom, 56) })}
    </p>
  {/if}

  {#if store.error}
    <p
      class="rounded-xl border border-risk-danger/30 p-4 text-sm text-risk-danger"
    >
      {store.error}
    </p>
  {/if}

  {#if store.step === "idle"}
    <section
      class="flex flex-wrap items-center gap-4 rounded-xl border border-surface-2 bg-surface-1 p-5"
      aria-label={lastScan ? t("cleaner.previousScan") : undefined}
    >
      {#if lastScan}
        <div class="min-w-0 flex-1">
          <p class="text-sm text-ink-muted">
            {t("dashboard.lastScan", { when: formatDateTime(lastScan.startedAt), count: formatCount(lastScan.findingCount) })}
            · {t("dashboard.couldFree", { size: formatBytes(lastScan.reclaimableBytes) })}
            {#if lastScan.status === "cancelled"} · {t("dashboard.wasCancelled")}{/if}
          </p>
          <p class="mt-1 text-xs leading-relaxed text-ink-faint">{t("cleaner.historyNotice")}</p>
        </div>
      {/if}
      <div class="flex items-center gap-2" class:ml-auto={!!lastScan}>
        <button
          type="button"
          class="rounded-lg bg-accent-soft px-4 py-2 text-sm font-medium text-ink transition-colors hover:bg-accent hover:text-surface-0"
          onclick={() => store.scan(MODULE_ID)}
        >
          {t("cleaner.scan")}
        </button>
        <InfoTip text={t("cleaner.intro")} />
      </div>
    </section>
  {:else if store.step === "scanning"}
    <section class="flex flex-col gap-3 rounded-xl border border-surface-2 bg-surface-1 p-6">
      <p class="text-sm">{t("cleaner.scanning")}</p>
      <ProgressBar label={t("cleaner.scanning")} />
      <p class="font-mono text-xs text-ink-faint">
        {store.currentPath ? truncatePath(store.currentPath, 70) : "…"}
      </p>
      <p class="text-xs text-ink-faint">
        {t("cleaner.itemsScanned", { count: store.itemsScanned })}
      </p>
    </section>
  {:else if store.step === "executing"}
    <section class="flex flex-col gap-3 rounded-xl border border-surface-2 bg-surface-1 p-6">
      <p class="text-sm">{t("cleaner.executing")}</p>
      <ProgressBar label={t("cleaner.executing")} />
    </section>
  {:else if store.step === "done" && store.report}
    <section class="flex flex-col items-start gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6">
      <h2 class="text-lg font-semibold tracking-tight">
        {t("cleaner.doneTitle")}
      </h2>
      <p class="text-sm text-ink-muted">
        {t("cleaner.doneBody", {
          size: formatBytes(store.report.reclaimedBytes),
        })}
      </p>

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
          class="rounded-lg bg-accent-soft px-4 py-2 text-sm font-medium text-ink transition-colors hover:bg-accent hover:text-surface-0"
          onclick={() => quarantineStore.show()}
        >
          {t("cleaner.openQuarantine")}
        </button>
        <button
          type="button"
          class="rounded-lg px-4 py-2 text-sm text-ink-muted transition-colors hover:bg-surface-2 hover:text-ink"
          onclick={resetScan}
        >
          {t("cleaner.scanAgain")}
        </button>
      </div>
    </section>
  {:else if store.step === "review" && store.summary}
    <section class="flex flex-wrap items-center gap-4 rounded-xl border border-surface-2 bg-surface-1 px-5 py-4">
      <p class="flex-1 text-sm leading-relaxed">
        {t("cleaner.summary", {
          safe: formatBytes(store.summary.safeBytes),
          review: formatBytes(store.summary.reviewBytes),
        })}
      </p>
      <button
        type="button"
        class="rounded-lg px-3 py-1.5 text-sm text-ink-muted transition-colors hover:bg-surface-2 hover:text-ink"
        onclick={resetScan}
      >
        {t("cleaner.scanAgain")}
      </button>
    </section>

    {#if store.result && store.result.skippedRules.length > 0}
      <p class="rounded-xl border border-surface-2 p-4 text-xs text-ink-faint">
        {t("cleaner.skippedRules", {
          rules: store.result.skippedRules.join(", "),
        })}
      </p>
    {/if}

    <section class="flex flex-col gap-3">
      <h2 class="flex items-center gap-2 text-sm font-medium">
        <RiskBadge risk="safe" count={safeRows.length} />
        <span class="text-ink-muted">{t("cleaner.safeHint")}</span>
      </h2>
      <FindingList
        risk="safe"
        rows={safeRows}
        selected={store.selected}
        ontoggle={(id, next) => store.toggle(id, next)}
        ontogglegroup={(ids, next) => store.toggleGroup(ids, next)}
      />
    </section>

    <section class="flex flex-col gap-3">
      <h2 class="flex items-center gap-2 text-sm font-medium">
        <RiskBadge risk="needsReview" count={reviewRows.length} />
        <span class="text-ink-muted">{t("cleaner.reviewHint")}</span>
      </h2>
      <FindingList
        risk="needsReview"
        rows={reviewRows}
        selected={store.selected}
        ontoggle={(id, next) => store.toggle(id, next)}
        ontogglegroup={(ids, next) => store.toggleGroup(ids, next)}
      />
    </section>

    <section class="flex flex-col gap-3">
      <h2 class="flex items-center gap-2 text-sm font-medium">
        <RiskBadge risk="doNotDelete" count={blockedRows.length} />
        <span class="text-ink-muted">{t("cleaner.blockedHint")}</span>
      </h2>
      <FindingList
        risk="doNotDelete"
        rows={blockedRows}
        selected={store.selected}
        ontoggle={() => {}}
        ontogglegroup={() => {}}
      />
    </section>

    <div
      class="sticky bottom-0 -mx-8 flex items-center gap-4 border-t border-surface-2 bg-surface-0/95 px-8 py-4 backdrop-blur"
    >
      <p class="flex-1 text-sm">
        {t("cleaner.selection", {
          count: store.selected.size,
          size: formatBytes(store.selectedBytes),
        })}
      </p>
      <button
        type="button"
        class="rounded-lg bg-accent-soft px-4 py-2 text-sm font-medium text-ink transition-colors hover:bg-accent hover:text-surface-0 disabled:cursor-not-allowed disabled:opacity-40"
        disabled={store.selected.size === 0}
        onclick={() => (confirming = true)}
      >
        {t("cleaner.clean")}
      </button>
    </div>
  {/if}
</div>

<ConfirmDialog
  open={confirming}
  title={t("cleaner.confirmTitle", { count: store.selected.size })}
  message={t("cleaner.confirmBody", {
    count: store.selected.size,
    size: formatBytes(store.selectedBytes),
  })}
  note={t("cleaner.confirmNote", { days: retentionDays })}
  warning={emptiesRecycleBin ? t("cleaner.confirmRecycleBin") : undefined}
  confirmLabel={t("cleaner.clean")}
  onconfirm={confirmExecute}
  oncancel={() => (confirming = false)}
/>
