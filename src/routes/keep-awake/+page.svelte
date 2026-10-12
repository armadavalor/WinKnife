<!--
  Keep-awake route: presets, optional screen-on, countdown, Stop always visible.

  Stop is never hidden behind a state check (module spec 06). Nothing here is
  persisted: the page tells the user so.
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";

  import InfoTip from "$lib/components/InfoTip.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { formatDateTime, formatRemaining } from "$lib/format";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { keepAwakeStore } from "$lib/stores/keepAwake.svelte";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { shutdownStore } from "$lib/stores/shutdown.svelte";

  const MODULE_ID = "keep-awake";
  const MAX_MINUTES = 1440;

  const manifest = $derived(moduleStore.find(MODULE_ID));
  const store = keepAwakeStore;

  let display = $state(false);
  let custom = $state("");
  let customInvalid = $state(false);

  const presets: { label: string; minutes: number }[] = [
    { label: t("keepAwake.preset30m"), minutes: 30 },
    { label: t("keepAwake.preset1h"), minutes: 60 },
    { label: t("keepAwake.preset2h"), minutes: 120 },
    { label: t("keepAwake.preset4h"), minutes: 240 },
  ];

  onMount(() => {
    void store.refresh();
    void shutdownStore.refresh();
  });

  $effect(() => {
    const unlisten = untrack(() => store.listen());
    return () => {
      void unlisten.then((stop) => stop());
    };
  });

  function startCustom(): void {
    const minutes = Number(custom);
    customInvalid = !Number.isInteger(minutes) || minutes < 1 || minutes > MAX_MINUTES;
    if (!customInvalid) {
      void store.start(minutes, display);
    }
  }
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(manifest.id, manifest.name) : MODULE_ID}
    info={manifest ? tModuleDescription(manifest.id, manifest.description) : undefined}
  />

  {#if shutdownStore.isArmed}
    <p
      class="rounded-xl border border-risk-review/30 p-4 text-sm leading-relaxed text-risk-review"
    >
      {t("keepAwake.shutdownArmed")}
    </p>
  {/if}

  {#if store.error}
    <p class="rounded-xl border border-risk-danger/30 p-4 text-sm text-risk-danger">
      {store.error}
    </p>
  {/if}

  {#if store.status.active && store.status.deadline}
    <section
      class="flex flex-col items-center gap-3 rounded-xl border border-surface-2 bg-surface-1 px-6 py-8"
    >
      <p class="text-xs uppercase tracking-wider text-ink-faint">
        {t("keepAwake.activeFor")}
      </p>
      <p class="text-5xl font-semibold tabular-nums tracking-tight">
        {formatRemaining(store.remainingSecs)}
      </p>
      <p class="text-sm text-ink-muted">
        {t("keepAwake.until", { time: formatDateTime(store.status.deadline) })}
      </p>
      {#if store.status.display}
        <p class="text-sm text-ink-muted">{t("keepAwake.activeDisplay")}</p>
      {/if}
      <button
        type="button"
        class="mt-2 rounded-lg bg-surface-2 px-4 py-2 text-sm text-ink transition-colors hover:bg-surface-3 disabled:cursor-not-allowed disabled:opacity-40"
        disabled={store.busy}
        onclick={() => store.stop()}
      >
        {t("keepAwake.stop")}
      </button>
    </section>
  {:else}
    <p class="flex items-center gap-2 text-sm text-ink-muted">
      <span class="size-2 rounded-full bg-ink-faint" aria-hidden="true"></span>
      {t("keepAwake.idle")}
    </p>
  {/if}

  <section class="flex flex-col gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6">
    <label class="flex items-center gap-2 text-sm text-ink">
      <input type="checkbox" bind:checked={display} />
      {t("keepAwake.displayLabel")}
    </label>

    <div class="flex flex-col gap-2">
      <span class="text-xs font-medium uppercase tracking-wider text-ink-faint">
        {t("keepAwake.presetsLabel")}
      </span>
      <div class="flex flex-wrap gap-2">
        {#each presets as preset (preset.minutes)}
          <button
            type="button"
            class="rounded-lg bg-surface-2 px-3 py-1.5 text-sm text-ink-muted transition-colors hover:bg-surface-3 hover:text-ink disabled:cursor-not-allowed disabled:opacity-40"
            disabled={store.busy}
            onclick={() => store.start(preset.minutes, display)}
          >
            {preset.label}
          </button>
        {/each}
      </div>
    </div>

    <div class="flex flex-col gap-2">
      <label
        class="text-xs font-medium uppercase tracking-wider text-ink-faint"
        for="custom-minutes"
      >
        {t("keepAwake.customLabel")}
      </label>
      <div class="flex items-center gap-2">
        <input
          id="custom-minutes"
          type="number"
          min="1"
          max={MAX_MINUTES}
          step="1"
          bind:value={custom}
          class="w-28 rounded-lg border border-surface-3 bg-surface-0 px-3 py-1.5 text-sm text-ink"
        />
        <button
          type="button"
          class="rounded-lg bg-accent-soft px-3 py-1.5 text-sm font-medium text-ink transition-colors hover:bg-accent hover:text-surface-0 disabled:cursor-not-allowed disabled:opacity-40"
          disabled={store.busy}
          onclick={startCustom}
        >
          {t("keepAwake.start")}
        </button>
        <InfoTip text={t("keepAwake.noPersist")} />
      </div>
      {#if customInvalid}
        <p class="text-xs text-risk-danger">{t("keepAwake.invalidMinutes")}</p>
      {/if}
    </div>
  </section>
</div>
