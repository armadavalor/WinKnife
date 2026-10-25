<!--
  Startup manager: everything that runs when you log in.

  A table rather than the three-bucket review the cleaner uses, because the
  question here is per-row and permanent — "should this run at logon?" — not
  "which of these should I remove today?".

  Two rules the table keeps:

  - Windows' own entries are greyed with the toggle disabled and the reason
    spelled out. That is module spec 05's red-bucket equivalent, and it is
    enforced in Rust as well: the executor refuses one even if the UI sent it.
  - The impact column always says "estimated", because it is a heuristic from
    the size of the program and the known-app list, not a boot-time measurement.
-->
<script lang="ts">
  import LoadingNote from "$lib/components/LoadingNote.svelte";
  import { onMount } from "svelte";

  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import ListToolbar from "$lib/components/ListToolbar.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { formatBytes, truncatePath } from "$lib/format";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { matchesFilter, sortRows } from "$lib/listTools";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { startupStore } from "$lib/stores/startup.svelte";
  import type { RiskBucket, StartupEntry } from "$lib/types";

  const MODULE_ID = "startup";
  const manifest = $derived(moduleStore.find(MODULE_ID));
  const store = startupStore;

  let pending = $state<StartupEntry | null>(null);
  let filter = $state("");

  // "size" is not meaningful here, so the sort control only offers name; the
  // view-only filter never changes what a toggle may do.
  const shownEntries = $derived(
    sortRows(
      store.entries.filter((entry) =>
        matchesFilter(filter, entry.name, entry.publisher ?? "", entry.command),
      ),
      "name",
      () => 0,
      (entry) => entry.name,
    ),
  );

  // Run-once mounting, deliberately not reactive: as an effect this subscribed
  // to the store's own `status` and re-enumerated on every transition.
  onMount(() => {
    void store.load();
  });

  const impactLabel: Record<StartupEntry["impact"], string> = {
    low: t("startup.impactLow"),
    medium: t("startup.impactMedium"),
    high: t("startup.impactHigh"),
  };

  const verdictLabel: Record<StartupEntry["verdict"], string> = {
    keep: t("startup.verdictKeep"),
    "user-choice": t("startup.verdictUserChoice"),
    "safe-to-disable": t("startup.verdictSafeToDisable"),
    unknown: t("startup.verdictUnknown"),
  };

  // Rust's rating for an entry it offers to turn off; colours match RiskBadge.
  const ratingLabel: Record<RiskBucket, string> = {
    safe: t("startup.ratingSafe"),
    needsReview: t("startup.ratingReview"),
    doNotDelete: t("startup.verdictKeep"),
  };
  const ratingStyle: Record<RiskBucket, string> = {
    safe: "text-risk-safe",
    needsReview: "text-risk-review",
    doNotDelete: "text-risk-danger",
  };

  function toggle(entry: StartupEntry): void {
    if (entry.osOwned || store.busy !== null || store.status === "loading") {
      return;
    }
    if (entry.enabled) {
      pending = entry;
    } else {
      void store.enable(entry);
    }
  }

  async function confirmDisable(): Promise<void> {
    const entry = pending;
    pending = null;
    if (entry) {
      await store.disable(entry);
    }
  }

  /** Enter and Space toggle the focused row, so the list is usable by keyboard. */
  function onRowKey(event: KeyboardEvent, entry: StartupEntry): void {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      toggle(entry);
    }
  }
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(manifest.id, manifest.name) : MODULE_ID}
    info={[manifest ? tModuleDescription(manifest.id, manifest.description) : "", t("startup.intro")]
      .filter(Boolean)
      .join(" ")}
  />
  <!-- Module spec 09: a plain link above the toggles. No modal, no nagging. -->
  <a
    href="/restore-point"
    class="self-start text-sm text-accent underline-offset-2 hover:underline"
  >
    {t("startup.restorePointLink")}
  </a>

  {#if store.error}
    <p class="rounded-xl border border-risk-danger/30 bg-surface-1 p-4 text-sm text-risk-danger">
      {store.error}
    </p>
  {/if}

  {#if store.status === "loading" || store.status === "idle"}
    <LoadingNote text={t("startup.loading")} />
  {:else if store.status === "error"}
    <div class="flex flex-col items-start gap-3 rounded-xl border border-surface-3 bg-surface-1 p-5">
      <p class="text-sm">{t("startup.loadError")}</p>
      <button
        type="button"
        class="btn-secondary px-3! py-1.5!"
        disabled={store.busy !== null}
        onclick={() => store.reload()}
      >
        {t("dashboard.retry")}
      </button>
    </div>
  {:else if store.entries.length === 0}
    <EmptyState message={t("startup.empty")} />
  {:else}
    <div class="flex flex-wrap items-center gap-3 text-sm text-ink-muted">
      <span>{t("startup.summary", { total: store.entries.length, on: store.enabledCount })}</span>
      <button
        type="button"
        class="btn-outline px-2.5! py-1! text-xs!"
        disabled={store.busy !== null}
        onclick={() => store.reload()}
      >
        {t("startup.refresh")}
      </button>
    </div>

    {#if store.unreadable.length > 0}
      <p class="rounded-lg bg-surface-2 p-3 text-xs leading-relaxed text-ink-muted">
        {t("startup.unreadable")}
        <span class="font-mono">{store.unreadable.join(" · ")}</span>
      </p>
    {/if}

    <ListToolbar
      bind:filter
      shown={shownEntries.length}
      total={store.entries.length}
    />

    {#if shownEntries.length === 0}
      <EmptyState message={t("list.noMatch")} />
    {/if}

    <ul class="flex flex-col gap-2">
      {#each shownEntries as entry (entry.id)}
        {@const rating = store.ratingOf(entry)}
        <li
          class="flex flex-wrap items-start gap-4 rounded-xl border border-surface-2 bg-surface-1 p-4 {entry.osOwned
            ? 'opacity-60'
            : ''}"
        >
          <div class="min-w-0 flex-1">
            <p class="flex flex-wrap items-center gap-2">
              <span class="font-medium">{entry.name}</span>
              {#if entry.signed}
                <span
                  class="rounded-md bg-surface-3 px-1.5 py-0.5 text-[0.65rem] text-ink-muted"
                  title={entry.signatureNote}
                >
                  ✓ {entry.publisher ?? t("startup.signed")}
                </span>
              {:else}
                <span
                  class="rounded-md border border-risk-warning/40 px-1.5 py-0.5 text-[0.65rem] text-risk-warning"
                  title={entry.signatureNote}
                >
                  {t("startup.unsigned")}
                </span>
              {/if}
              {#if entry.category}
                <span class="rounded-md bg-surface-2 px-1.5 py-0.5 text-[0.65rem] text-ink-faint">
                  {entry.category}
                </span>
              {/if}
            </p>
            <p class="mt-1 truncate font-mono text-[0.7rem] text-ink-faint" title={entry.command}>
              {entry.location} · {truncatePath(entry.command, 64)}
            </p>
            <p class="mt-1.5 text-xs leading-relaxed text-ink-muted">{entry.note}</p>
            {#if rating}
              <details class="group mt-1.5">
                <summary class="cursor-pointer text-xs text-ink-faint hover:text-ink">
                  {t("startup.ratingWhy")}
                </summary>
                <p class="mt-1 text-xs leading-relaxed text-ink-muted">{rating.reason}</p>
              </details>
            {/if}
            {#if entry.leftoverShortcut}
              <details class="group mt-1.5">
                <summary class="cursor-pointer text-xs text-ink-faint hover:text-ink">
                  {t("startup.leftoverSummary")}
                </summary>
                <p class="mt-1 text-xs leading-relaxed text-risk-warning">
                  {t("startup.leftoverReason", { path: entry.leftoverShortcut })}
                </p>
              </details>
            {/if}
            {#if entry.osOwned && entry.osReason && entry.osReason !== entry.note}
              <p class="mt-1.5 text-xs leading-relaxed text-ink-faint">{entry.osReason}</p>
            {/if}
          </div>

          <div class="flex shrink-0 flex-col items-end gap-1.5">
            <span class="text-[0.7rem] text-ink-faint" title={entry.impactReason}>
              {t("startup.impact", { tier: impactLabel[entry.impact] })}
            </span>
            {#if rating}
              <span class="text-[0.7rem] font-medium {ratingStyle[rating.risk]}">
                {ratingLabel[rating.risk]}
              </span>
            {:else}
              <span class="text-[0.7rem] text-ink-faint">{verdictLabel[entry.verdict]}</span>
            {/if}
            <button
              type="button"
              role="switch"
              aria-checked={entry.enabled}
              aria-label={t("startup.toggleLabel", { name: entry.name })}
              disabled={entry.osOwned || store.busy !== null}
              class="rounded-lg px-3 py-1 text-xs font-medium transition-colors {entry.enabled
                ? 'bg-accent-soft text-ink hover:bg-accent hover:text-on-accent'
                : 'border border-surface-3 text-ink-muted hover:bg-surface-2'} disabled:cursor-not-allowed disabled:opacity-50"
              onclick={() => toggle(entry)}
              onkeydown={(event) => onRowKey(event, entry)}
            >
              {store.busy === entry.id
                ? t("startup.working")
                : entry.enabled
                  ? t("startup.on")
                  : entry.leftoverShortcut
                    ? t("startup.offLeftover")
                    : t("startup.off")}
            </button>
          </div>
        </li>
      {/each}
    </ul>

    {#if store.lastReport}
      <p class="rounded-xl bg-surface-2/60 p-4 text-sm text-ink-muted">
        {store.lastReport}
      </p>
    {/if}
  {/if}
</div>

<ConfirmDialog
  open={pending !== null}
  title={t("startup.confirmTitle", { name: pending?.name ?? "" })}
  message={t("startup.confirmBody")}
  note={t("startup.confirmNote")}
  confirmLabel={t("startup.confirmAction")}
  onconfirm={confirmDisable}
  oncancel={() => (pending = null)}
/>
