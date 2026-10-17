<!--
  Duplicates route: the same Scan → Review → Execute shape as the cleaner, with
  the differences module spec 03 asks for.

  Sets are sorted by wasted bytes, every copy shows its dates and the keeper
  carries a badge, and nothing starts selected — a duplicate is a judgement
  about a pair, and only a person can make it.
-->
<script lang="ts">
  import Icon from "$lib/components/Icon.svelte";
  import { page } from "$app/state";
  import { onMount } from "svelte";

  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { formatBytes, formatDateTime, truncatePath } from "$lib/format";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { duplicatesStore } from "$lib/stores/duplicates.svelte";
  import { itemsToReport, warningsOf } from "$lib/stores/execNotes";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { quarantineStore } from "$lib/stores/quarantine.svelte";
  import { settingsStore } from "$lib/stores/settings.svelte";
  import type { ExtensionClass } from "$lib/types";

  const MODULE_ID = "duplicates";

  const manifest = $derived(moduleStore.find(MODULE_ID));
  const store = duplicatesStore;

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
  });

  let expanded = $state<Set<string>>(new Set());

  // `?root=` is the treemap's "Find duplicates here" cross-link arriving with a
  // folder already chosen (module spec 04). It narrows this scan only; the saved
  // roots on the settings page are untouched.
  $effect(() => {
    store.pinnedRoot = page.url.searchParams.get("root");
  });

  const sets = $derived(store.sets);
  const scan = $derived(store.result?.scan ?? null);

  const CLASSES: { value: ExtensionClass; label: string }[] = [
    { value: "all", label: t("duplicates.classAll") },
    { value: "images", label: t("duplicates.classImages") },
    { value: "video", label: t("duplicates.classVideo") },
    { value: "documents", label: t("duplicates.classDocuments") },
    { value: "other", label: t("duplicates.classOther") },
  ];

  const MIN_SIZES = [
    { value: 0, label: t("duplicates.minSize.any") },
    { value: 1024 * 1024, label: "1 MB" },
    { value: 10 * 1024 * 1024, label: "10 MB" },
    { value: 100 * 1024 * 1024, label: "100 MB" },
    { value: 1024 * 1024 * 1024, label: "1 GB" },
  ];

  function toggleExpanded(setId: string): void {
    const next = new Set(expanded);
    if (next.has(setId)) {
      next.delete(setId);
    } else {
      next.add(setId);
    }
    expanded = next;
  }

  async function confirmExecute(): Promise<void> {
    confirming = false;
    await store.execute();
    await quarantineStore.reload();
  }
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(manifest.id, manifest.name) : MODULE_ID}
    info={manifest ? tModuleDescription(manifest.id, manifest.description) : undefined}
  />

  {#if store.error}
    <p
      class="rounded-xl border border-risk-danger/30 bg-surface-1 p-4 text-sm text-risk-danger"
    >
      {store.error}
    </p>
  {/if}

  {#if store.step === "idle"}
    <section
      class="flex flex-col items-start gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6"
    >
      {#if store.pinnedRoot}
        <p
          class="flex flex-wrap items-center gap-2 rounded-lg bg-surface-2 px-3 py-2 text-xs text-ink-muted"
        >
          <span>{t("duplicates.pinnedRoot", { path: truncatePath(store.pinnedRoot, 56) })}</span>
          <button
            type="button"
            class="rounded px-1.5 py-0.5 text-ink-faint underline-offset-2 hover:text-ink hover:underline"
            onclick={() => (store.pinnedRoot = null)}
          >
            {t("duplicates.clearPinnedRoot")}
          </button>
        </p>
      {/if}
      <div class="flex items-center gap-2">
        <button
          type="button"
          class="btn-primary"
          onclick={() => store.scan()}
        >
          {t("duplicates.scan")}
        </button>
      </div>
      <p class="text-sm text-ink-muted">{t("duplicates.intro")}</p>
      <details class="group">
        <summary class="cursor-pointer text-xs text-ink-faint hover:text-ink">
          {t("duplicates.how")}
        </summary>
        <p class="mt-2 text-xs leading-relaxed text-ink-muted">{t("duplicates.howBody")}</p>
      </details>
    </section>
  {:else if store.step === "scanning"}
    <section
      class="flex flex-col gap-3 rounded-xl border border-surface-2 bg-surface-1 p-6"
    >
      <p class="text-sm">{t("duplicates.scanning")}</p>
      <ProgressBar
        label={t("duplicates.scanning")}
        pct={store.pct}
      />
      <p class="font-mono text-xs text-ink-faint">
        {store.currentPath ? truncatePath(store.currentPath, 70) : "…"}
      </p>
      <button
        type="button"
        class="self-start btn-ghost px-3! py-1.5!"
        disabled={store.stopping}
        onclick={() => store.stop()}
      >
        {store.stopping ? t("duplicates.stopping") : t("duplicates.stop")}
      </button>
    </section>
  {:else if store.step === "executing"}
    <section
      class="flex flex-col gap-3 rounded-xl border border-surface-2 bg-surface-1 p-6"
    >
      <p class="text-sm">{t("cleaner.executing")}</p>
      <ProgressBar label={t("cleaner.executing")} />
    </section>
  {:else if store.step === "done" && store.report}
    <section
      class="flex flex-col items-start gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6"
    >
      <h2 class="text-lg font-semibold tracking-tight">
        {t("duplicates.doneTitle")}
      </h2>
      <p class="text-sm text-ink-muted">
        {t("duplicates.doneBody", {
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
          class="btn-primary"
          onclick={() => quarantineStore.show()}
        >
          {t("cleaner.openQuarantine")}
        </button>
        <button
          type="button"
          class="btn-ghost"
          onclick={() => store.reset()}
        >
          {t("duplicates.scanAgain")}
        </button>
      </div>
    </section>
  {:else if store.step === "review" && scan}
    <section
      class="flex flex-wrap items-center gap-4 rounded-xl border border-surface-2 bg-surface-1 px-5 py-4"
    >
      <p class="flex-1 text-sm leading-relaxed">
        {t("duplicates.summary", {
          size: formatBytes(store.result?.wastedBytes ?? 0),
          sets: scan.sets.length,
        })}
        <span class="text-ink-faint">
          · {t("duplicates.hashed", { count: scan.filesHashed })}
        </span>
      </p>
      <button
        type="button"
        class="btn-ghost px-3! py-1.5!"
        onclick={() => store.reset()}
      >
        {t("duplicates.scanAgain")}
      </button>
    </section>

    {#if scan.cancelled}
      <p class="rounded-xl bg-surface-2/60 p-4 text-sm text-ink-muted">
        {t("duplicates.cancelled")}
      </p>
    {/if}

    {#if scan.skippedRoots.length > 0}
      <p class="rounded-xl bg-surface-2/60 p-4 text-sm text-ink-muted">
        {t("duplicates.skippedRoots", { roots: scan.skippedRoots.join(" · ") })}
      </p>
    {/if}

    <section class="flex flex-wrap items-center gap-3 text-sm">
      <label class="flex items-center gap-2">
        <span class="text-ink-muted">{t("duplicates.filterClass")}</span>
        <select
          class="rounded-lg border border-surface-3 bg-surface-0 px-2 py-1 text-sm"
          bind:value={store.filterClass}
        >
          {#each CLASSES as option (option.value)}
            <option value={option.value}>{option.label}</option>
          {/each}
        </select>
      </label>

      <label class="flex items-center gap-2">
        <span class="text-ink-muted">{t("duplicates.filterMinSize")}</span>
        <select
          class="rounded-lg border border-surface-3 bg-surface-0 px-2 py-1 text-sm"
          bind:value={store.filterMinBytes}
        >
          {#each MIN_SIZES as option (option.value)}
            <option value={option.value}>{option.label}</option>
          {/each}
        </select>
      </label>

      <span class="flex-1"></span>

      <button
        type="button"
        class="btn-ghost px-3! py-1.5! text-xs!"
        onclick={() => store.keepBy("oldest")}
      >
        {t("duplicates.keepOldest")}
      </button>
      <button
        type="button"
        class="btn-ghost px-3! py-1.5! text-xs!"
        onclick={() => store.keepBy("newest")}
      >
        {t("duplicates.keepNewest")}
      </button>
      <button
        type="button"
        class="btn-ghost px-3! py-1.5! text-xs!"
        onclick={() => store.selectAllNonKeepers()}
      >
        {t("duplicates.selectAll")}
      </button>
      <button
        type="button"
        class="btn-ghost px-3! py-1.5! text-xs!"
        onclick={() => store.selectNone()}
      >
        {t("duplicates.selectNone")}
      </button>
    </section>

    {#if sets.length === 0}
      <EmptyState message={t("duplicates.empty")} />
    {:else}
      <div class="flex flex-col gap-3">
        {#each sets as set (set.id)}
          {@const open = expanded.has(set.id)}
          <section class="rounded-xl border border-surface-2 bg-surface-1">
            <header class="flex items-center gap-3 px-4 py-3">
              <button
                type="button"
                class="flex flex-1 items-center gap-2 text-left"
                aria-expanded={open}
                aria-controls="set-{set.id}"
                onclick={() => toggleExpanded(set.id)}
              >
                <Icon
              name="chevron-right"
              class="size-4 shrink-0 text-ink-muted transition-transform duration-150"
              style={open ? "transform: rotate(90deg)" : undefined}
            />
                <span class="min-w-0 flex-1">
                  <span class="block truncate font-mono text-xs text-ink">
                    {truncatePath(set.copies[0]?.path ?? "", 56)}
                  </span>
                  <span class="mt-0.5 block text-xs text-ink-muted">
                    {t("duplicates.setLabel", {
                      count: set.copies.length,
                      size: formatBytes(set.sizeBytes),
                    })}
                  </span>
                </span>
              </button>
              <span
                class="shrink-0 text-xs tabular-nums text-risk-review"
                title={t("duplicates.wasted", {
                  size: formatBytes(set.wastedBytes),
                })}
              >
                {formatBytes(set.wastedBytes)}
              </span>
            </header>

            <ul
              id="set-{set.id}"
              class="divide-y divide-surface-2 border-t border-surface-2"
              class:hidden={!open}
            >
              {#each set.copies as copy (copy.findingId)}
                {@const keeper = store.isKeeper(set, copy)}
                {@const rec = store.recommendationFor(copy)}
                <li class="flex items-start gap-3 px-4 py-3">
                  {#if keeper || !rec || rec.action !== "quarantine"}
                    <span class="mt-1 size-4 shrink-0" aria-hidden="true"></span>
                  {:else}
                    <input
                      type="checkbox"
                      class="mt-1 size-4 shrink-0 accent-[var(--color-accent)]"
                      checked={store.selected.has(rec.id)}
                      aria-label={copy.path}
                      onchange={(event) =>
                        store.toggle(rec.id, event.currentTarget.checked)}
                    />
                  {/if}

                  <span class="min-w-0 flex-1">
                    <span class="flex items-center gap-2">
                      <span
                        class="min-w-0 flex-1 truncate font-mono text-xs text-ink"
                        title={copy.path}
                      >
                        {truncatePath(copy.path)}
                      </span>
                      {#if keeper}
                        <span
                          class="shrink-0 rounded-md bg-risk-safe/15 px-1.5 py-0.5 text-[0.65rem] font-medium text-risk-safe"
                        >
                          {t("duplicates.keeper")}
                        </span>
                      {/if}
                    </span>
                    <span
                      class="mt-1 block text-xs leading-relaxed text-ink-muted"
                    >
                      {keeper
                        ? copy.keeperReason
                        : (rec?.reason ?? copy.keeperReason)}
                    </span>
                    <span class="mt-1 block text-[0.7rem] text-ink-faint">
                      {copy.modified
                        ? formatDateTime(copy.modified)
                        : t("duplicates.noDate")}
                      {#if !keeper}
                        ·
                        <button
                          type="button"
                          class="underline underline-offset-2 transition-colors hover:text-ink"
                          onclick={() => store.chooseKeeper(set, copy)}
                        >
                          {t("duplicates.makeKeeper")}
                        </button>
                      {/if}
                    </span>
                  </span>
                </li>
              {/each}
            </ul>
          </section>
        {/each}
      </div>
    {/if}

    {#if scan.linkedNames.length > 0}
      <section class="flex flex-col gap-2 rounded-xl border border-surface-2 p-4">
        <h2 class="text-sm font-medium">{t("duplicates.linkedTitle")}</h2>
        {#each scan.linkedNames as link (link.path)}
          <p class="text-xs leading-relaxed text-ink-faint">{link.note}</p>
        {/each}
      </section>
    {/if}

    <div
      class="sticky bottom-0 -mx-8 flex items-center gap-4 border-t border-surface-2 bg-surface-0/95 px-8 py-4 backdrop-blur"
    >
      <p class="flex-1 text-sm">
        {t("duplicates.selection", {
          count: store.selected.size,
          size: formatBytes(store.selectedBytes),
        })}
      </p>
      <button
        type="button"
        class="btn-primary"
        disabled={store.selected.size === 0}
        onclick={() => (confirming = true)}
      >
        {t("duplicates.remove")}
      </button>
    </div>
  {/if}
</div>

<ConfirmDialog
  open={confirming}
  title={t("duplicates.confirmTitle", { count: store.selected.size })}
  message={t("duplicates.confirmBody", {
    size: formatBytes(store.selectedBytes),
  })}
  note={t("cleaner.confirmNote", { days: retentionDays })}
  confirmLabel={t("duplicates.remove")}
  onconfirm={confirmExecute}
  oncancel={() => (confirming = false)}
/>
