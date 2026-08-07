<!--
  Dashboard: what this machine looks like right now.

  Every number here comes from something that actually happened — a scan that
  ran, a batch that is held, a timer that is armed. A module that has never been
  run says so and offers to run; it does not show a zero, because a zero reads
  as "nothing to find" rather than "nobody has looked".
-->
<script lang="ts">
  import { onMount } from "svelte";

  import ModuleIcon from "$lib/components/ModuleIcon.svelte";
  import { formatBytes, formatDateTime, formatRemaining } from "$lib/format";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { dashboardStore } from "$lib/stores/dashboard.svelte";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { quarantineStore } from "$lib/stores/quarantine.svelte";
  import type { ModuleManifest } from "$lib/types";

  const store = dashboardStore;

  // Run-once mounting, deliberately not reactive: as an effect this subscribed
  // to the store's own `status`, so "ready" was never observable and the
  // reclaimable figure below stayed an em dash.
  onMount(() => {
    void store.load();
  });

  function actionLabel(module: ModuleManifest): string {
    switch (module.id) {
      case "cleaner":
        return t("cleaner.scan");
      case "treemap":
        return t("treemap.scan");
      case "duplicates":
        return t("duplicates.scan");
      case "startup":
        return t("dashboard.review");
      default:
        return t("module.open");
    }
  }

  const timer = $derived(store.summary?.timer.active ?? null);
  const drives = $derived(store.summary?.drives ?? []);
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-8 px-8 py-10">
  <header class="flex flex-col gap-2">
    <h1 class="text-2xl font-semibold tracking-tight">{t("dashboard.title")}</h1>
    <p class="max-w-2xl text-sm leading-relaxed text-ink-muted">
      {t("dashboard.subtitle")}
    </p>
  </header>

  {#if store.status === "error"}
    <div class="flex flex-col items-start gap-3 rounded-xl border border-surface-3 bg-surface-1 p-5">
      <p class="text-sm">{t("dashboard.error")}</p>
      {#if store.error}
        <p class="font-mono text-xs text-ink-faint">{store.error}</p>
      {/if}
      <button
        type="button"
        class="rounded-lg bg-surface-3 px-3 py-1.5 text-sm transition-colors hover:bg-accent-soft"
        onclick={() => store.load()}
      >
        {t("dashboard.retry")}
      </button>
    </div>
  {/if}

  <!-- The three things worth knowing at a glance -->
  <section class="grid grid-cols-1 gap-3 sm:grid-cols-3">
    <div class="rounded-xl border border-surface-2 bg-surface-1 p-4">
      <p class="text-xs uppercase tracking-wider text-ink-faint">
        {t("dashboard.reclaimable")}
      </p>
      <p class="mt-1 text-xl font-semibold tabular-nums">
        {store.status === "ready" ? formatBytes(store.reclaimable) : "—"}
      </p>
      <p class="mt-1 text-xs leading-relaxed text-ink-muted">
        {t("dashboard.reclaimableNote")}
      </p>
    </div>

    <div class="rounded-xl border border-surface-2 bg-surface-1 p-4">
      <p class="text-xs uppercase tracking-wider text-ink-faint">
        {t("quarantine.title")}
      </p>
      <p class="mt-1 text-xl font-semibold tabular-nums">
        {store.summary
          ? formatBytes(store.summary.quarantine.totalBytes)
          : "—"}
      </p>
      <button
        type="button"
        class="mt-1 text-xs text-ink-muted underline-offset-2 hover:text-ink hover:underline"
        onclick={() => quarantineStore.show()}
      >
        {t("dashboard.quarantineNote", {
          count: store.summary?.quarantine.itemCount ?? 0,
        })}
      </button>
    </div>

    <div class="rounded-xl border border-surface-2 bg-surface-1 p-4">
      <p class="text-xs uppercase tracking-wider text-ink-faint">
        {t("dashboard.timer")}
      </p>
      {#if timer}
        <p class="mt-1 text-xl font-semibold tabular-nums">
          {formatRemaining(timer.remainingSecs)}
        </p>
        <a
          href="/shutdown-timer"
          class="mt-1 inline-block text-xs text-ink-muted underline-offset-2 hover:text-ink hover:underline"
        >
          {t("dashboard.timerActive", { when: formatDateTime(timer.target) })}
        </a>
      {:else}
        <p class="mt-1 text-xl font-semibold text-ink-faint">—</p>
        <a
          href="/shutdown-timer"
          class="mt-1 inline-block text-xs text-ink-muted underline-offset-2 hover:text-ink hover:underline"
        >
          {t("shutdown.noTimer")}
        </a>
      {/if}
    </div>
  </section>

  {#if drives.length > 0}
    <section class="flex flex-wrap gap-3 text-xs text-ink-muted">
      {#each drives as drive (drive.root)}
        <a
          href="/treemap"
          class="rounded-lg border border-surface-2 px-3 py-2 transition-colors hover:border-accent-soft"
        >
          <span class="font-medium text-ink">{drive.root}</span>
          {formatBytes(drive.freeBytes)} {t("dashboard.free")}
        </a>
      {/each}
    </section>
  {/if}

  <!-- One card per module -->
  {#if moduleStore.status === "loading" || moduleStore.status === "idle"}
    <p class="text-sm text-ink-faint">{t("dashboard.loading")}</p>
  {:else if moduleStore.modules.length === 0}
    <p class="text-sm text-ink-faint">{t("dashboard.empty")}</p>
  {:else}
    <ul class="grid grid-cols-1 gap-4 sm:grid-cols-2">
      {#each moduleStore.modules as module (module.id)}
        {@const last = store.lastScan(module.id)}
        <li>
          <div
            class="flex h-full flex-col gap-3 rounded-xl border border-surface-2 bg-surface-1 p-5"
          >
            <span class="flex items-center gap-3">
              <ModuleIcon name={module.icon} class="size-5 text-accent" />
              <span class="font-medium">
                {tModuleName(module.id, module.name)}
              </span>
            </span>

            <span class="text-sm leading-relaxed text-ink-muted">
              {tModuleDescription(module.id, module.description)}
            </span>

            <span class="text-xs text-ink-faint">
              {#if last}
                {t("dashboard.lastScan", {
                  when: formatDateTime(last.startedAt),
                  count: last.findingCount,
                })}
                {#if last.reclaimableBytes > 0}
                  · {t("dashboard.couldFree", {
                    size: formatBytes(last.reclaimableBytes),
                  })}
                {/if}
                {#if last.status === "cancelled"}
                  · {t("dashboard.wasCancelled")}
                {/if}
              {:else if module.capabilities.hasScan}
                {t("dashboard.neverRun")}
              {:else}
                {t("dashboard.noScan")}
              {/if}
            </span>

            <a
              href="/{module.id}"
              class="mt-auto inline-block w-fit rounded-lg bg-accent-soft px-3 py-1.5 text-sm font-medium transition-colors hover:bg-accent hover:text-surface-0"
            >
              {actionLabel(module)}
            </a>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</div>
