<!--
  Tweaks: a few per-user Windows settings, each with an exact undo (K-018).

  Every row shows the registry path, the value now and the value after, and
  where the setting is documented, before anything changes. Applying records
  the prior value first; Undo puts that back.
-->
<script lang="ts">
  import LoadingNote from "$lib/components/LoadingNote.svelte";
  import { onMount } from "svelte";

  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { tweaksUpdateStatus } from "$lib/api";
  import { tweaksStore } from "$lib/stores/tweaks.svelte";
  import {
    isApiError,
    type TweakCategory,
    type TweakRestart,
    type TweakState,
    type UpdateStatus,
  } from "$lib/types";

  const MODULE_ID = "tweaks";
  const manifest = $derived(moduleStore.find(MODULE_ID));
  const store = tweaksStore;

  let pending = $state<TweakState | null>(null);
  let updateStatus = $state<UpdateStatus | null>(null);
  let updateError = $state<string | null>(null);

  async function loadUpdateStatus(): Promise<void> {
    updateError = null;
    try {
      updateStatus = await tweaksUpdateStatus();
    } catch (error: unknown) {
      updateStatus = null;
      updateError = isApiError(error) ? error.message : String(error);
    }
  }

  function refresh(): void {
    void store.reload();
    void loadUpdateStatus();
  }

  onMount(() => {
    void store.load();
    void loadUpdateStatus();
  });

  const restartLabel: Record<TweakRestart, string> = {
    none: t("tweaks.restartNone"),
    explorer: t("tweaks.restartExplorer"),
    reboot: t("tweaks.restartReboot"),
  };

  const CATEGORY_ORDER: TweakCategory[] = ["explorer", "taskbarStart", "privacy", "input", "appearance"];
  let query = $state("");

  const groups = $derived.by(() => {
    const needle = query.trim().toLowerCase();
    const shown = store.tweaks.filter(
      (tw) =>
        !needle ||
        `${tw.name} ${tw.reason} ${tw.path}`.toLowerCase().includes(needle),
    );
    return CATEGORY_ORDER.map((category) => ({
      category,
      items: shown.filter((tw) => tw.category === category),
    })).filter((group) => group.items.length > 0);
  });

  const explorerNote = $derived(
    store.tweaks.some((tw) => tw.applied && tw.restart === "explorer"),
  );

  function stateOf(tweak: TweakState): "applied" | "differs" | "default" {
    if (tweak.applied) return "applied";
    return !tweak.error && !store.recommendationOf(tweak) ? "differs" : "default";
  }

  function valueText(value: number | null): string {
    return value === null ? t("tweaks.notSet") : String(value);
  }

  async function confirmApply(): Promise<void> {
    const tweak = pending;
    pending = null;
    if (tweak) {
      await store.applyTweak(tweak);
    }
  }
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(manifest.id, manifest.name) : MODULE_ID}
    info={[manifest ? tModuleDescription(manifest.id, manifest.description) : "", t("tweaks.intro")]
      .filter(Boolean)
      .join(" ")}
  />

  {#if store.error}
    <p class="rounded-xl border border-risk-danger/30 bg-surface-1 p-4 text-sm text-risk-danger">
      {store.error}
    </p>
  {/if}

  {#if store.status === "loading" || store.status === "idle"}
    <LoadingNote text={t("tweaks.loading")} />
  {:else if store.status === "error"}
    <div class="flex flex-col items-start gap-3 rounded-xl border border-surface-3 bg-surface-1 p-5">
      <p class="text-sm">{t("tweaks.loadError")}</p>
      <button
        type="button"
        class="btn-secondary px-3! py-1.5!"
        disabled={store.busy !== null}
        onclick={() => store.reload()}
      >
        {t("dashboard.retry")}
      </button>
    </div>
  {:else if store.tweaks.length === 0}
    <EmptyState message={t("tweaks.empty")} />
  {:else}
    <div class="flex flex-wrap items-center gap-3">
      <input
        type="search"
        class="min-w-0 flex-1 rounded-lg border border-surface-3 bg-surface-1 px-3 py-1.5 text-sm"
        placeholder={t("tweaks.search")}
        aria-label={t("tweaks.search")}
        bind:value={query}
      />
      <button
        type="button"
        class="btn-outline px-2.5! py-1! text-xs!"
        disabled={store.busy !== null}
        onclick={refresh}
      >
        {t("tweaks.refresh")}
      </button>
    </div>

    {#if explorerNote}
      <p class="rounded-xl bg-surface-2/60 p-3 text-xs text-ink-muted">{t("tweaks.explorerNote")}</p>
    {/if}

    {#if groups.length === 0}
      <EmptyState message={t("tweaks.noMatch", { query })} />
    {/if}

    {#each groups as group (group.category)}
    <section class="flex flex-col gap-2">
    <h2 class="text-sm font-semibold text-ink-muted">{t(`tweaks.category.${group.category}`)}</h2>
    <ul class="flex flex-col gap-2">
      {#each group.items as tweak (tweak.id)}
        {@const recommendation = store.recommendationOf(tweak)}
        {@const state = stateOf(tweak)}
        <li class="flex flex-wrap items-start gap-4 rounded-xl border border-surface-2 bg-surface-1 p-4">
          <div class="min-w-0 flex-1">
            <p class="flex flex-wrap items-center gap-2">
              <span class="font-medium">{tweak.name}</span>
              <span class="rounded-md bg-surface-2 px-1.5 py-0.5 text-[0.65rem] text-ink-faint">
                {restartLabel[tweak.restart]}
              </span>
              <span
                class="rounded-md px-1.5 py-0.5 text-[0.65rem] {state === 'applied'
                  ? 'bg-accent/15 text-accent'
                  : 'bg-surface-2 text-ink-faint'}"
              >
                {t(`tweaks.state.${state}`)}
              </span>
            </p>
            <p class="mt-1 select-text break-all font-mono text-[0.7rem] text-ink-faint">
              {tweak.path}
            </p>
            <p class="mt-1.5 text-xs text-ink-muted">
              {t("tweaks.now")}: <span class="font-mono">{valueText(tweak.current)}</span>
              · {t("tweaks.after")}: <span class="font-mono">{tweak.target}</span>
            </p>
            {#if tweak.error}
              <p class="mt-1.5 text-xs leading-relaxed text-risk-warning">{tweak.error}</p>
            {/if}
            {#if tweak.drift}
              <p class="mt-1.5 text-xs leading-relaxed text-risk-warning">{tweak.drift}</p>
            {/if}
            <details class="group mt-1.5">
              <summary class="cursor-pointer text-xs text-ink-faint hover:text-ink">
                {t("tweaks.why")}
              </summary>
              <p class="mt-1 text-xs leading-relaxed text-ink-muted">{tweak.reason}</p>
              <!-- No opener plugin: the link is selectable text, never a navigation. -->
              <p class="mt-1 text-xs text-ink-faint">
                {t("tweaks.appliesTo")}: {tweak.appliesTo}
              </p>
              <p class="mt-1 text-xs text-ink-faint">
                {t("tweaks.source")}
                <span class="select-text break-all font-mono">{tweak.sourceUrl}</span>
              </p>
            </details>
          </div>

          <div class="flex shrink-0 flex-col items-end gap-1.5">
            <span class="text-[0.7rem] text-ink-faint">
              {tweak.undoability === "reversibleAfterRestart"
                ? t("tweaks.reversibleAfterRestart")
                : t("tweaks.reversible")}
            </span>
            {#if tweak.applied}
              <button
                type="button"
                class="btn-secondary px-3! py-1! text-xs!"
                disabled={store.busy !== null}
                onclick={() => store.undo(tweak, t("tweaks.undone"))}
              >
                {store.busy === tweak.id ? t("tweaks.working") : t("tweaks.undo")}
              </button>
            {:else if recommendation}
              <button
                type="button"
                class="btn-primary px-3! py-1! text-xs!"
                disabled={store.busy !== null}
                onclick={() => (pending = tweak)}
              >
                {store.busy === tweak.id
                  ? t("tweaks.working")
                  : tweak.drift
                    ? t("tweaks.reapply")
                    : t("tweaks.apply")}
              </button>
            {:else if !tweak.error}
              <span class="text-[0.7rem] text-ink-muted">{t("tweaks.done")}</span>
            {/if}
          </div>
        </li>
      {/each}
    </ul>
    </section>
    {/each}

    <section class="rounded-xl border border-surface-2 bg-surface-1 p-4">
      <h2 class="text-sm font-medium">{t("tweaks.updateTitle")}</h2>
      <p class="mt-1 text-xs text-ink-muted">{t("tweaks.updateIntro")}</p>
      {#if updateError}
        <p class="mt-2 text-xs text-risk-warning">{updateError}</p>
      {:else if updateStatus === null}
        <p class="mt-2 text-xs text-ink-faint">{t("tweaks.loading")}</p>
      {:else}
        {#if updateStatus.blocks.length === 0 && updateStatus.errors.length === 0}
          <p class="mt-2 text-xs text-ink-muted">{t("tweaks.updateClear")}</p>
        {:else}
          <ul class="mt-2 flex flex-col gap-2">
            {#each updateStatus.blocks as block (block.path)}
              <li class="text-xs">
                <p class="leading-relaxed text-risk-warning">{block.reason}</p>
                <p class="mt-0.5 select-text break-all font-mono text-[0.7rem] text-ink-faint">
                  {block.path} = {block.value}
                </p>
              </li>
            {/each}
          </ul>
        {/if}
        {#each updateStatus.errors as error (error)}
          <p class="mt-1.5 break-all text-xs text-risk-warning">{t("tweaks.updateUnread")} {error}</p>
        {/each}
      {/if}
    </section>

    {#if store.lastReport}
      <p class="rounded-xl bg-surface-2/60 p-4 text-sm text-ink-muted">
        {store.lastReport}
      </p>
    {/if}
  {/if}
</div>

<ConfirmDialog
  open={pending !== null}
  title={t("tweaks.confirmTitle", { name: pending?.name ?? "" })}
  message={t("tweaks.confirmBody", {
    from: valueText(pending?.current ?? null),
    to: String(pending?.target ?? ""),
  })}
  note={t("tweaks.confirmNote")}
  detail={pending?.path}
  confirmLabel={t("tweaks.confirmAction")}
  onconfirm={confirmApply}
  oncancel={() => (pending = null)}
/>
