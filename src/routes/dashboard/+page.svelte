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
  import { pinnedModules } from "$lib/pinnedModules";
  import { t, tModuleName } from "$lib/i18n";
  import { dashboardStore } from "$lib/stores/dashboard.svelte";
  import { pinnedModulesStore } from "$lib/stores/pinnedModules.svelte";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { quarantineStore } from "$lib/stores/quarantine.svelte";

  const store = dashboardStore;

  // Run-once mounting, deliberately not reactive: as an effect this subscribed
  // to the store's own `status`, so "ready" was never observable and the
  // reclaimable figure below stayed an em dash.
  onMount(() => {
    void store.load();
  });

  const timer = $derived(store.summary?.timer.active ?? null);
  const drives = $derived(store.summary?.drives ?? []);

  const pinned = $derived(pinnedModules(moduleStore.modules, pinnedModulesStore.ids));
  const unpinned = $derived(moduleStore.modules.filter((m) => !pinnedModulesStore.ids.includes(m.id)));
  let picking = $state(false);

  const glass =
    "rounded-2xl border border-surface-3/60 bg-surface-1/[var(--panel-alpha,60%)] backdrop-blur-xl";
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

  <!-- Tools the user pinned; the sidebar lists every tool -->
  <section class="flex flex-col gap-3">
    <h2 class="text-xs font-medium uppercase tracking-wider text-ink-muted">{t("dashboard.pinned")}</h2>
    {#if moduleStore.status === "loading" || moduleStore.status === "idle"}
      <LoadingNote text={t("dashboard.loading")} />
    {:else}
      <ul class="grid grid-cols-2 gap-3 sm:grid-cols-3">
        {#each pinned as module (module.id)}
          {@const name = tModuleName(module.id, module.name)}
          <li class="group relative">
            <a
              href="/{module.id}"
              class="flex items-center gap-3 {glass} px-4 py-3 pr-9 text-sm font-medium transition-colors hover:bg-surface-1/85"
            >
              <ModuleIcon name={module.icon} class="size-5 shrink-0 text-accent" />
              <span class="truncate">{name}</span>
            </a>
            <button
              type="button"
              class="absolute right-2 top-1/2 -translate-y-1/2 rounded-md px-1.5 text-ink-faint opacity-0 transition-opacity hover:text-ink focus-visible:opacity-100 group-hover:opacity-100"
              aria-label={t("dashboard.unpin", { name })}
              title={t("dashboard.unpin", { name })}
              onclick={() => pinnedModulesStore.toggle(module.id)}
            >×</button>
          </li>
        {/each}
        <li>
          <button
            type="button"
            class="flex w-full items-center justify-center gap-2 {glass} border-dashed px-4 py-3 text-sm font-medium text-ink-muted transition-colors hover:bg-surface-1/85 hover:text-ink"
            aria-haspopup="dialog"
            aria-expanded={picking}
            onclick={() => (picking = !picking)}
          >
            <span aria-hidden="true">+</span>
            {t("dashboard.addTool")}
          </button>
        </li>
      </ul>
      {#if picking}
        <div
          class="{glass} p-4"
          role="dialog"
          aria-label={t("dashboard.addTool")}
          tabindex="-1"
          onkeydown={(e) => e.key === "Escape" && (picking = false)}
        >
          {#if unpinned.length === 0}
            <p class="text-sm text-ink-faint">{t("dashboard.allPinned")}</p>
          {:else}
            <ul class="grid grid-cols-2 gap-2 sm:grid-cols-3">
              {#each unpinned as module (module.id)}
                <li>
                  <button
                    type="button"
                    class="flex w-full items-center gap-2 rounded-xl px-3 py-2 text-left text-sm transition-colors hover:bg-surface-2/80"
                    onclick={() => pinnedModulesStore.toggle(module.id)}
                  >
                    <ModuleIcon name={module.icon} class="size-4 shrink-0 text-accent" />
                    <span class="truncate">{tModuleName(module.id, module.name)}</span>
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
          <button type="button" class="btn-secondary mt-3" onclick={() => (picking = false)}>
            {t("dashboard.done")}
          </button>
        </div>
      {/if}
    {/if}
  </section>
</div>
