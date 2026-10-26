<!--
  Scheduled scan route (C11): the schedule, the green-only auto-quarantine
  opt-in, and the last report with a way back into the cleaner's review.

  The run itself is started from the shell (`+layout.svelte`), not here, so a
  slot fires whichever page is open.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import { goto } from "$app/navigation";

  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { formatBytes, formatCount, formatDateTime } from "$lib/format";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { locale } from "$lib/stores/locale.svelte";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { scanStore } from "$lib/stores/scan.svelte";
  import { scheduledScanStore } from "$lib/stores/scheduledScan.svelte";
  import type { AutoQuarantineSkip, BucketTotals, ScheduleFrequency } from "$lib/types";

  const MODULE_ID = "scheduled-scan";
  // 2024-01-01 was a Monday; the backend counts weekdays from Monday = 0.
  const KNOWN_MONDAY_UTC = Date.UTC(2024, 0, 1);
  const DAYS_IN_WEEK = 7;
  const SKIP_KEYS = {
    busy: "scheduledScan.autoSkipped.busy",
    cancelled: "scheduledScan.autoSkipped.cancelled",
    failed: "scheduledScan.autoSkipped.failed",
    manualReview: "scheduledScan.autoSkipped.manualReview",
  } as const satisfies Record<AutoQuarantineSkip, string>;

  const manifest = $derived(moduleStore.find(MODULE_ID));
  const store = scheduledScanStore;

  let kind = $state<ScheduleFrequency["kind"]>("off");
  let weekday = $state(0);
  let time = $state("09:00");
  let autoQuarantine = $state(false);
  /** Turning auto-quarantine on waits for this dialog (K-031: explicit consent). */
  let confirmingAuto = $state(false);
  let saved = $state(false);
  let gone = $state(false);

  const weekdays = $derived.by(() => {
    const format = new Intl.DateTimeFormat(locale.current, { weekday: "long", timeZone: "UTC" });
    return Array.from({ length: DAYS_IN_WEEK }, (_, i) =>
      format.format(new Date(KNOWN_MONDAY_UTC + i * 86_400_000)),
    );
  });

  const report = $derived(store.state?.lastReport ?? null);

  onMount(() => {
    void store.refresh().then(() => {
      const settings = store.state?.settings;
      if (!settings) return;
      kind = settings.frequency.kind;
      weekday = settings.frequency.kind === "weekly" ? settings.frequency.weekday : 0;
      time = `${pad(settings.hour)}:${pad(settings.minute)}`;
      autoQuarantine = settings.autoQuarantine;
    });
  });

  function pad(n: number): string {
    return String(n).padStart(2, "0");
  }

  function frequency(): ScheduleFrequency {
    if (kind === "weekly") return { kind, weekday };
    return { kind };
  }

  async function save(): Promise<void> {
    const [hour, minute] = time.split(":").map(Number);
    saved = await store.save({
      frequency: frequency(),
      hour: hour ?? 0,
      minute: minute ?? 0,
      autoQuarantine,
      armedAt: store.state?.settings.armedAt ?? null,
    });
  }

  async function review(scanId: string): Promise<void> {
    gone = false;
    try {
      await scanStore.loadExisting(scanId);
      await goto("/cleaner");
    } catch {
      gone = true;
    }
  }

  function bucket(totals: BucketTotals): string {
    return t("scheduledScan.bucketLine", {
      count: formatCount(totals.count),
      bytes: formatBytes(totals.bytes),
    });
  }
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(manifest.id, manifest.name) : MODULE_ID}
    info={manifest ? tModuleDescription(manifest.id, manifest.description) : undefined}
  />

  {#if store.error}
    <p class="rounded-xl border border-risk-danger/30 bg-surface-1 p-4 text-sm text-risk-danger">
      {store.error}
    </p>
  {/if}

  <section class="flex flex-col gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6">
    <h2 class="text-sm font-semibold">{t("scheduledScan.scheduleTitle")}</h2>
    <div class="flex flex-wrap items-end gap-4 text-sm">
      <label class="flex flex-col gap-1">
        <span class="text-ink-muted">{t("scheduledScan.frequency")}</span>
        <select
          class="rounded-lg border border-surface-3 bg-surface-0 px-2 py-1 text-sm"
          bind:value={kind}
          onchange={() => (saved = false)}
        >
          <option value="off">{t("scheduledScan.off")}</option>
          <option value="daily">{t("scheduledScan.daily")}</option>
          <option value="weekly">{t("scheduledScan.weekly")}</option>
        </select>
      </label>
      {#if kind === "weekly"}
        <label class="flex flex-col gap-1">
          <span class="text-ink-muted">{t("scheduledScan.weekday")}</span>
          <select
            class="rounded-lg border border-surface-3 bg-surface-0 px-2 py-1 text-sm"
            bind:value={weekday}
            onchange={() => (saved = false)}
          >
            {#each weekdays as name, i (i)}
              <option value={i}>{name}</option>
            {/each}
          </select>
        </label>
      {/if}
      {#if kind !== "off"}
        <label class="flex flex-col gap-1">
          <span class="text-ink-muted">{t("scheduledScan.time")}</span>
          <input
            type="time"
            class="rounded-lg border border-surface-3 bg-surface-0 px-2 py-1 text-sm"
            bind:value={time}
            oninput={() => (saved = false)}
          />
        </label>
      {/if}
    </div>

    <label class="flex items-start gap-3 text-sm">
      <input
        type="checkbox"
        class="mt-1"
        checked={autoQuarantine}
        onchange={(event) => {
          saved = false;
          if (event.currentTarget.checked) {
            // Stays off until the dialog is confirmed; Cancel leaves it off.
            event.currentTarget.checked = false;
            confirmingAuto = true;
          } else {
            autoQuarantine = false;
          }
        }}
      />
      <span class="flex flex-col gap-1">
        <span class="font-medium">{t("scheduledScan.autoTitle")}</span>
        <span class="text-ink-muted">{t("scheduledScan.autoDetail")}</span>
      </span>
    </label>

    <p class="text-xs leading-relaxed text-ink-faint">{t("scheduledScan.windowsNote")}</p>

    <div class="flex items-center gap-3">
      <button type="button" class="btn-primary" disabled={store.saving} onclick={() => void save()}>
        {t("scheduledScan.save")}
      </button>
      {#if saved}
        <span class="text-sm text-risk-safe" role="status">{t("scheduledScan.saved")}</span>
      {/if}
    </div>
  </section>

  <section class="flex flex-col gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6">
    <h2 class="text-sm font-semibold">{t("scheduledScan.lastTitle")}</h2>
    {#if report}
      <p class="text-sm text-ink-muted">
        {t("scheduledScan.ranAt", { time: formatDateTime(report.ranAt) })}
      </p>
      {#if report.cancelled}
        <p class="text-sm text-risk-review">{t("scheduledScan.cancelledRun")}</p>
      {/if}
      <dl class="grid grid-cols-1 gap-3 text-sm sm:grid-cols-3">
        <div class="rounded-lg border border-surface-2 p-3">
          <dt class="text-risk-safe">{t("scheduledScan.bucketSafe")}</dt>
          <dd>{bucket(report.safe)}</dd>
        </div>
        <div class="rounded-lg border border-surface-2 p-3">
          <dt class="text-risk-review">{t("scheduledScan.bucketReview")}</dt>
          <dd>{bucket(report.review)}</dd>
        </div>
        <div class="rounded-lg border border-surface-2 p-3">
          <dt class="text-risk-danger">{t("scheduledScan.bucketBlocked")}</dt>
          <dd>{bucket(report.blocked)}</dd>
        </div>
      </dl>
      {#if report.autoQuarantine?.kind === "ran"}
        <p class="text-sm">
          {t("scheduledScan.autoRan", {
            moved: formatCount(report.autoQuarantine.moved),
            bytes: formatBytes(report.autoQuarantine.bytes),
          })}
        </p>
        {#if report.autoQuarantine.notMoved > 0}
          <p class="text-sm text-ink-muted">
            {t("scheduledScan.autoNotMoved", { count: formatCount(report.autoQuarantine.notMoved) })}
          </p>
        {/if}
      {:else if report.autoQuarantine?.kind === "skipped"}
        <p class="text-sm text-ink-muted">
          {t(SKIP_KEYS[report.autoQuarantine.reason])}
        </p>
      {/if}
      <div>
        <button type="button" class="btn-secondary" onclick={() => void review(report.scanId)}>
          {t("scheduledScan.review")}
        </button>
      </div>
      {#if gone}
        <p class="text-sm text-risk-review" role="status">{t("scheduledScan.gone")}</p>
      {/if}
    {:else}
      <p class="text-sm text-ink-muted">{t("scheduledScan.none")}</p>
    {/if}
  </section>
</div>

<ConfirmDialog
  open={confirmingAuto}
  title={t("scheduledScan.autoConfirmTitle")}
  message={t("scheduledScan.autoConfirmBody")}
  note={t("scheduledScan.autoConfirmNote")}
  confirmLabel={t("scheduledScan.autoConfirmAction")}
  onconfirm={() => {
    autoQuarantine = true;
    confirmingAuto = false;
  }}
  oncancel={() => (confirmingAuto = false)}
/>
