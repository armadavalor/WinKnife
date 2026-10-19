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
  import StatCard from "$lib/components/StatCard.svelte";
  import {
    formatBytes,
    formatCount,
    formatDateTime,
    formatRemaining,
  } from "$lib/format";
  import { t, tDynamic, tModuleDescription, tModuleName } from "$lib/i18n";
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
    return tDynamic(`dashboard.action.${module.id}`, t("module.open"));
  }

  function statusText(module: ModuleManifest): string {
    return tDynamic(`dashboard.status.${module.id}`, t("dashboard.status.default"));
  }

  const timer = $derived(store.summary?.timer.active ?? null);
  const drives = $derived(store.summary?.drives ?? []);
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader title={t("dashboard.title")} info={t("dashboard.subtitle")} />

  {#if store.status === "error"}
    <div class="flex flex-col items-start gap-3 rounded-xl border border-surface-3 bg-surface-1 p-5">
      <p class="text-sm">{t("dashboard.error")}</p>
      {#if store.error}
        <p class="font-mono text-xs text-ink-faint">{store.error}</p>
      {/if}
      <button
        type="button"
        class="btn-secondary"
        onclick={() => store.load()}
      >
        {t("dashboard.retry")}
      </button>
    </div>
  {/if}

  <!-- The three things worth knowing at a glance -->
  <section class="grid grid-cols-1 gap-3 sm:grid-cols-3">
    <StatCard
      label={t("dashboard.reclaimable")}
      value={store.status === "ready" ? formatBytes(store.reclaimable) : "—"}
      info={t("dashboard.reclaimableNote")}
    />

    <StatCard
      label={t("quarantine.title")}
      value={store.summary ? formatBytes(store.summary.quarantine.totalBytes) : "—"}
    >
      {#snippet footer()}
        <button
          type="button"
          class="mt-1 text-xs text-ink-muted underline-offset-2 hover:text-ink hover:underline"
          onclick={() => quarantineStore.show()}
        >
          {t("dashboard.quarantineNote", {
            count: store.summary?.quarantine.itemCount ?? 0,
          })}
        </button>
      {/snippet}
    </StatCard>

    <StatCard
      label={t("dashboard.timer")}
      value={timer ? formatRemaining(timer.remainingSecs) : "—"}
    >
      {#snippet footer()}
        <a
          href="/shutdown-timer"
          class="mt-1 inline-block text-xs text-ink-muted underline-offset-2 hover:text-ink hover:underline"
        >
          {timer
            ? t("dashboard.timerActive", { when: formatDateTime(timer.target) })
            : t("shutdown.noTimer")}
        </a>
      {/snippet}
    </StatCard>
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
    <LoadingNote text={t("dashboard.loading")} />
  {:else if moduleStore.modules.length === 0}
    <p class="text-sm text-ink-faint">{t("dashboard.empty")}</p>
  {:else}
    <ul class="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
      {#each moduleStore.modules as module (module.id)}
        {@const last = store.lastScan(module.id)}
        <li>
          <div
            class="flex h-full flex-col gap-3 rounded-xl border border-surface-2 bg-surface-1 p-4"
          >
            <span class="flex items-center gap-2">
              <ModuleIcon name={module.icon} class="size-5 shrink-0 text-accent" />
              <a href="/{module.id}" class="font-medium hover:underline">
                {tModuleName(module.id, module.name)}
              </a>
              <InfoTip text={tModuleDescription(module.id, module.description)} />
            </span>

            <span class="text-xs text-ink-muted">
              {#if last}
                {t("dashboard.lastScan", {
                  when: formatDateTime(last.startedAt),
                  count: formatCount(last.findingCount),
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
                {statusText(module)}
              {/if}
            </span>

            <a
              href="/{module.id}"
              class="btn-primary mt-auto w-fit"
            >
              {actionLabel(module)}
            </a>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</div>
