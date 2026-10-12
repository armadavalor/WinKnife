<!--
  Shutdown timer route: big countdown, quick presets, cancel always visible.

  The cancel button is never hidden behind a state check — a user who wants to
  call off a shutdown should not have to think about which screen they are on
  (module spec 01).
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";

  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import InfoTip from "$lib/components/InfoTip.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { formatDateTime, formatRemaining } from "$lib/format";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { shutdownStore } from "$lib/stores/shutdown.svelte";
  import type { PowerAction, ScheduleMode } from "$lib/types";

  const MODULE_ID = "shutdown-timer";

  const manifest = $derived(moduleStore.find(MODULE_ID));
  const store = shutdownStore;

  let action = $state<PowerAction>("shutdown");
  let clockTime = $state("23:00");
  let pending = $state<ScheduleMode | null>(null);

  const presets: { label: string; mode: ScheduleMode }[] = [
    { label: t("shutdown.preset30m"), mode: { mode: "in", minutes: 30 } },
    { label: t("shutdown.preset1h"), mode: { mode: "in", minutes: 60 } },
    { label: t("shutdown.preset2h"), mode: { mode: "in", minutes: 120 } },
    { label: t("shutdown.presetTonight"), mode: { mode: "at", hour: 23, minute: 0 } },
  ];

  const actions: { value: PowerAction; label: string }[] = [
    { value: "shutdown", label: t("shutdown.actionShutdown") },
    { value: "restart", label: t("shutdown.actionRestart") },
    { value: "sleep", label: t("shutdown.actionSleep") },
  ];

  // The first fetch is a one-shot and belongs on mount; only the subscription
  // needs an effect, and that one is untracked so nothing it reads on the way in
  // can re-run it.
  onMount(() => {
    void store.refresh();
  });

  $effect(() => {
    const unlisten = untrack(() => store.listen());
    return () => {
      void unlisten.then((stop) => stop());
    };
  });

  function request(mode: ScheduleMode): void {
    // Replacing an existing timer is fine, but it gets a confirmation:
    // silently swapping what the machine is about to do would be rude.
    if (store.isArmed) {
      pending = mode;
      return;
    }
    void store.schedule(mode, action);
  }

  function scheduleFromClock(): void {
    const [hour, minute] = clockTime.split(":").map(Number);
    if (Number.isNaN(hour) || Number.isNaN(minute)) {
      return;
    }
    request({ mode: "at", hour, minute });
  }

  async function confirmReplace(): Promise<void> {
    const mode = pending;
    pending = null;
    if (mode) {
      await store.schedule(mode, action);
    }
  }
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(manifest.id, manifest.name) : MODULE_ID}
    info={manifest ? tModuleDescription(manifest.id, manifest.description) : undefined}
  />

  {#if store.status.dryRun}
    <p
      class="rounded-xl border border-risk-review/30 p-4 text-sm leading-relaxed text-risk-review"
    >
      {t("shutdown.dryRunBanner")}
    </p>
  {/if}

  {#if store.error}
    <p
      class="rounded-xl border border-risk-danger/30 p-4 text-sm text-risk-danger"
    >
      {store.error}
    </p>
  {/if}

  <section
    class="flex flex-col items-center gap-3 rounded-xl border border-surface-2 bg-surface-1 px-6 py-10"
  >
    {#if store.status.active}
      <p class="text-xs uppercase tracking-wider text-ink-faint">
        {t(`shutdown.countdownFor.${store.status.active.action}`)}
      </p>
      <p class="text-5xl font-semibold tabular-nums tracking-tight">
        {formatRemaining(store.remainingSecs)}
      </p>
      <p class="text-sm text-ink-muted">
        {t("shutdown.willRunAt", {
          time: formatDateTime(store.status.active.target),
        })}
      </p>
    {:else}
      <p class="text-5xl font-semibold tabular-nums tracking-tight text-ink-faint">
        --:--
      </p>
      <p class="text-sm text-ink-muted">{t("shutdown.noTimer")}</p>
    {/if}

    <button
      type="button"
      class="mt-2 rounded-lg px-4 py-2 text-sm text-ink-muted transition-colors hover:bg-surface-2 hover:text-ink disabled:cursor-not-allowed disabled:opacity-40"
      disabled={!store.isArmed || store.busy}
      onclick={() => store.cancel()}
    >
      {t("shutdown.cancel")}
    </button>
  </section>

  <section class="flex flex-col gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6">
    <div class="flex flex-col gap-2">
      <span class="text-xs font-medium uppercase tracking-wider text-ink-faint">
        {t("shutdown.actionLabel")}
      </span>
      <div class="flex flex-wrap gap-2" role="radiogroup" aria-label={t("shutdown.actionLabel")}>
        {#each actions as option (option.value)}
          <button
            type="button"
            role="radio"
            aria-checked={action === option.value}
            class="rounded-lg px-3 py-1.5 text-sm transition-colors {action ===
            option.value
              ? 'bg-surface-3 text-ink'
              : 'text-ink-muted hover:bg-surface-2 hover:text-ink'}"
            onclick={() => (action = option.value)}
          >
            {option.label}
          </button>
        {/each}
      </div>
    </div>

    <div class="flex flex-col gap-2">
      <span class="text-xs font-medium uppercase tracking-wider text-ink-faint">
        {t("shutdown.presetsLabel")}
      </span>
      <div class="flex flex-wrap gap-2">
        {#each presets as preset (preset.label)}
          <button
            type="button"
            class="rounded-lg bg-surface-2 px-3 py-1.5 text-sm text-ink-muted transition-colors hover:bg-surface-3 hover:text-ink disabled:cursor-not-allowed disabled:opacity-40"
            disabled={store.busy}
            onclick={() => request(preset.mode)}
          >
            {preset.label}
          </button>
        {/each}
      </div>
    </div>

    <div class="flex flex-col gap-2">
      <label
        class="text-xs font-medium uppercase tracking-wider text-ink-faint"
        for="clock-time"
      >
        {t("shutdown.clockLabel")}
      </label>
      <div class="flex items-center gap-2">
        <input
          id="clock-time"
          type="time"
          bind:value={clockTime}
          class="rounded-lg border border-surface-3 bg-surface-0 px-3 py-1.5 text-sm text-ink"
        />
        <button
          type="button"
          class="rounded-lg bg-accent-soft px-3 py-1.5 text-sm font-medium text-ink transition-colors hover:bg-accent hover:text-surface-0 disabled:cursor-not-allowed disabled:opacity-40"
          disabled={store.busy}
          onclick={scheduleFromClock}
        >
          {t("shutdown.setTimer")}
        </button>
        <InfoTip text={t("shutdown.trayHint")} />
      </div>
    </div>
  </section>
</div>

<ConfirmDialog
  open={pending !== null}
  title={t("shutdown.replaceTitle")}
  message={t("shutdown.replaceBody")}
  confirmLabel={t("shutdown.setTimer")}
  onconfirm={confirmReplace}
  oncancel={() => (pending = null)}
/>
