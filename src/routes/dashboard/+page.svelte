<!--
  Dashboard: what this machine looks like right now.

  Every number here comes from something that actually happened — a scan that
  ran, a batch that is held, a timer that is armed. A module that has never been
  run says so and offers to run; it does not show a zero, because a zero reads
  as "nothing to find" rather than "nobody has looked".
-->
<script lang="ts">
  import LoadingNote from "$lib/components/LoadingNote.svelte";
  import { onMount } from "svelte";

  import InfoTip from "$lib/components/InfoTip.svelte";
  import ModuleIcon from "$lib/components/ModuleIcon.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { formatBytes, formatDateTime, formatRemaining } from "$lib/format";
  import { visibleModules } from "$lib/hiddenModules";
  import { t, tModuleName } from "$lib/i18n";
  import { dashboardStore } from "$lib/stores/dashboard.svelte";
  import { hiddenModules } from "$lib/stores/hiddenModules.svelte";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { quarantineStore } from "$lib/stores/quarantine.svelte";

  const store = dashboardStore;

  /** Tiles shown, and the tools offered before anything has been run. */
  const RECENT_COUNT = 6;
  const DEFAULT_TOOLS = ["cleaner", "treemap", "duplicates", "startup", "apps", "shutdown-timer"];

  // Run-once mounting, deliberately not reactive: as an effect this subscribed
  // to the store's own `status`, so "ready" was never observable and the
  // reclaimable figure below stayed an em dash.
  onMount(() => {
    void store.load();
  });

  const timer = $derived(store.summary?.timer.active ?? null);
  const drives = $derived(store.summary?.drives ?? []);

  // Most recently run first, then the defaults, then registry order.
  const recent = $derived.by(() => {
    const shown = visibleModules(moduleStore.modules, hiddenModules.ids);
    const ranTime = (id: string) => Date.parse(store.lastScan(id)?.startedAt ?? "") || 0;
    const ran = shown.filter((m) => ranTime(m.id) > 0).sort((a, b) => ranTime(b.id) - ranTime(a.id));
    const rest = shown.filter((m) => !ran.includes(m));
    const defaults = DEFAULT_TOOLS.flatMap((id) => rest.filter((m) => m.id === id));
    return [...ran, ...defaults, ...rest.filter((m) => !defaults.includes(m))].slice(0, RECENT_COUNT);
  });

  const glass =
    "rounded-2xl border border-surface-3/60 bg-surface-1/60 backdrop-blur-xl";
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader title={t("dashboard.title")} info={t("dashboard.subtitle")} />

  {#if store.status === "error"}
    <div class="flex flex-col items-start gap-3 {glass} p-5">
      <p class="text-sm">{t("dashboard.error")}</p>
      {#if store.error}
        <p class="font-mono text-xs text-ink-faint">{store.error}</p>
      {/if}
      <button type="button" class="btn-secondary" onclick={() => store.load()}>
        {t("dashboard.retry")}
      </button>
    </div>
  {/if}

  <!-- One card: the number worth knowing, the one thing worth doing -->
  <section class="flex flex-col gap-5 {glass} p-6">
    <div class="flex flex-wrap items-end justify-between gap-4">
      <div>
        <p class="flex items-center gap-1.5 text-xs uppercase tracking-wider text-ink-faint">
          <span>{t("dashboard.reclaimable")}</span>
          <InfoTip text={t("dashboard.reclaimableNote")} />
        </p>
        <p class="mt-1 text-4xl font-semibold tabular-nums">
          {store.status === "ready" ? formatBytes(store.reclaimable) : "—"}
        </p>
      </div>
      <a href="/cleaner" class="btn-primary">{t("cleaner.scan")}</a>
    </div>

    <div class="flex flex-wrap items-center gap-2 text-xs text-ink-muted">
      {#each drives as drive (drive.root)}
        <a
          href="/treemap"
          class="rounded-xl border border-surface-3/60 bg-surface-2/50 px-3 py-1.5 transition-colors hover:border-accent-soft hover:bg-surface-2/80"
        >
          <span class="font-medium text-ink">{drive.root}</span>
          {formatBytes(drive.freeBytes)} {t("dashboard.free")}
        </a>
      {/each}
      <button
        type="button"
        class="rounded-xl border border-surface-3/60 bg-surface-2/50 px-3 py-1.5 transition-colors hover:border-accent-soft hover:bg-surface-2/80"
        onclick={() => quarantineStore.show()}
      >
        <span class="font-medium text-ink">{t("quarantine.title")}</span>
        {store.summary ? formatBytes(store.summary.quarantine.totalBytes) : "—"} ·
        {t("dashboard.quarantineNote", { count: store.summary?.quarantine.itemCount ?? 0 })}
      </button>
      {#if timer}
        <a
          href="/shutdown-timer"
          class="rounded-xl border border-surface-3/60 bg-surface-2/50 px-3 py-1.5 transition-colors hover:border-accent-soft hover:bg-surface-2/80"
        >
          <span class="font-medium text-ink">{t("dashboard.timer")}</span>
          {formatRemaining(timer.remainingSecs)} · {t("dashboard.timerActive", { when: formatDateTime(timer.target) })}
        </a>
      {/if}
    </div>
  </section>

  <!-- The sidebar lists every tool; this is only the way back to the last few -->
  <section class="flex flex-col gap-3">
    <h2 class="text-xs font-medium uppercase tracking-wider text-ink-muted">{t("dashboard.recent")}</h2>
    {#if moduleStore.status === "loading" || moduleStore.status === "idle"}
      <LoadingNote text={t("dashboard.loading")} />
    {:else if recent.length === 0}
      <p class="text-sm text-ink-faint">{t("dashboard.empty")}</p>
    {:else}
      <ul class="grid grid-cols-2 gap-3 sm:grid-cols-3">
        {#each recent as module (module.id)}
          <li>
            <a
              href="/{module.id}"
              class="flex items-center gap-3 {glass} px-4 py-3 text-sm font-medium transition-colors hover:bg-surface-1/85"
            >
              <ModuleIcon name={module.icon} class="size-5 shrink-0 text-accent" />
              <span class="truncate">{tModuleName(module.id, module.name)}</span>
            </a>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
</div>
