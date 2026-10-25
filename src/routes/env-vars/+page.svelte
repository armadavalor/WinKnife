<!--
  Environment variables (C15, K-024).

  User variables (HKCU) can be added, edited and deleted; system ones (HKLM)
  are shown read-only. Every edit is a draft in this page: the backend diffs it
  read-only, and only "Save" after the preview writes. Each save records the
  prior value so Undo puts it back exactly, kind (REG_SZ / REG_EXPAND_SZ) included.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import { envVarsApply, envVarsList, envVarsPreview, envVarsUndo } from "$lib/api";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import LoadingNote from "$lib/components/LoadingNote.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import {
    isApiError,
    type EnvChange,
    type EnvEntryFlag,
    type EnvListing,
    type EnvPreview,
    type EnvValue,
    type EnvVar,
  } from "$lib/types";

  const MODULE_ID = "env-vars";
  const LIST_SEPARATOR = ";";
  const manifest = $derived(moduleStore.find(MODULE_ID));

  interface Row {
    text: string;
    expanded: string | null;
    flags: EnvEntryFlag[];
  }

  interface Draft {
    name: string;
    isNew: boolean;
    /** What the user was shown; the backend refuses if the live value moved. */
    expected: EnvValue | null;
    value: string;
    expand: boolean;
    listMode: boolean;
    rows: Row[];
  }

  type Status = "loading" | "ready" | "error";

  let listing = $state<EnvListing | null>(null);
  let status = $state<Status>("loading");
  let error = $state<string | null>(null);
  let busy = $state(false);
  let report = $state<string | null>(null);
  let draft = $state<Draft | null>(null);
  let pending = $state<{ change: EnvChange; preview: EnvPreview } | null>(null);

  const undoable = $derived(new Set((listing?.undo ?? []).map((u) => u.name.toUpperCase())));
  const deletedUndo = $derived((listing?.undo ?? []).filter((u) => u.deleted));

  onMount(() => {
    void load();
  });

  function message(cause: unknown): string {
    return isApiError(cause) ? cause.message : String(cause);
  }

  async function load(): Promise<void> {
    status = "loading";
    try {
      listing = await envVarsList();
      status = "ready";
    } catch (cause) {
      error = message(cause);
      status = "error";
    }
  }

  function draftValue(d: Draft): string {
    return d.listMode ? d.rows.map((r) => r.text).join(LIST_SEPARATOR) : d.value;
  }

  function plainRows(value: string): Row[] {
    return value.split(LIST_SEPARATOR).map((text) => ({ text, expanded: null, flags: [] }));
  }

  function startEdit(v: EnvVar): void {
    pending = null;
    error = null;
    draft = {
      name: v.name,
      isNew: false,
      expected: { value: v.value, expand: v.expand },
      value: v.value,
      expand: v.expand,
      listMode: v.isList,
      rows: v.entries.map((e) => ({ text: e.text, expanded: e.expanded, flags: e.flags })),
    };
  }

  function startAdd(): void {
    pending = null;
    error = null;
    draft = { name: "", isNew: true, expected: null, value: "", expand: false, listMode: false, rows: [] };
  }

  function toggleListMode(d: Draft): void {
    if (d.listMode) {
      d.value = draftValue(d);
    } else {
      d.rows = plainRows(d.value);
    }
    d.listMode = !d.listMode;
  }

  function moveRow(d: Draft, index: number, by: number): void {
    const target = index + by;
    if (target < 0 || target >= d.rows.length) return;
    const rows = [...d.rows];
    [rows[index], rows[target]] = [rows[target], rows[index]];
    d.rows = rows;
  }

  function editRow(d: Draft, index: number, text: string): void {
    // An edited entry has not been checked yet, so its old flags no longer apply.
    d.rows = d.rows.map((r, i) => (i === index ? { text, expanded: null, flags: [] } : r));
  }

  async function review(change: EnvChange): Promise<void> {
    busy = true;
    error = null;
    try {
      pending = { change, preview: await envVarsPreview(change) };
    } catch (cause) {
      error = message(cause);
    } finally {
      busy = false;
    }
  }

  function reviewDraft(d: Draft): void {
    void review({
      name: d.name.trim(),
      scope: "user",
      new: { value: draftValue(d), expand: d.expand },
      expected: d.expected,
    });
  }

  function reviewDelete(v: EnvVar): void {
    draft = null;
    void review({ name: v.name, scope: "user", new: null, expected: { value: v.value, expand: v.expand } });
  }

  async function save(): Promise<void> {
    const current = pending;
    if (!current) return;
    busy = true;
    error = null;
    try {
      await envVarsApply(current.change);
      report = t(current.change.new ? "envVars.saved" : "envVars.deleted", { name: current.change.name });
      pending = null;
      draft = null;
      await load();
    } catch (cause) {
      error = message(cause);
    } finally {
      busy = false;
    }
  }

  async function undo(name: string): Promise<void> {
    busy = true;
    error = null;
    try {
      await envVarsUndo(name);
      report = t("envVars.undone", { name });
      pending = null;
      draft = null;
      await load();
    } catch (cause) {
      error = message(cause);
    } finally {
      busy = false;
    }
  }

  function kindLabel(expand: boolean): string {
    return expand ? "REG_EXPAND_SZ" : "REG_SZ";
  }
</script>

{#snippet entryList(v: EnvVar)}
  <ol class="mt-2 flex flex-col gap-1">
    {#each v.entries as entry, index (index)}
      <li class="text-xs">
        <span class="select-text break-all font-mono text-[0.7rem]" class:text-ink-faint={!entry.text}>
          {entry.text || t("envVars.emptyEntry")}
        </span>
        {#if entry.expanded}
          <span class="select-text break-all font-mono text-[0.7rem] text-ink-faint">→ {entry.expanded}</span>
        {/if}
        {#each entry.flags as flag (flag.kind)}
          <p class="text-[0.7rem] text-risk-warning">{t(`envVars.flag.${flag.kind}`)}: {flag.reason}</p>
        {/each}
      </li>
    {/each}
  </ol>
{/snippet}

{#snippet variable(v: EnvVar)}
  {@const flagged = v.entries.filter((e) => e.flags.length > 0).length}
  <li class="flex flex-wrap items-start gap-4 rounded-xl border border-surface-2 bg-surface-1 p-4">
    <div class="min-w-0 flex-1">
      <p class="flex flex-wrap items-center gap-2">
        <span class="select-text font-medium font-mono text-sm">{v.name}</span>
        <span class="rounded-md bg-surface-2 px-1.5 py-0.5 text-[0.65rem] text-ink-faint">{kindLabel(v.expand)}</span>
        {#if !v.editable}
          <span class="rounded-md bg-surface-2 px-1.5 py-0.5 text-[0.65rem] text-ink-faint">{t("envVars.readOnly")}</span>
        {/if}
        {#if flagged > 0}
          <span class="text-[0.7rem] text-risk-warning">{t("envVars.flaggedCount", { count: flagged })}</span>
        {/if}
      </p>
      {#if v.readOnlyReason}
        <p class="mt-1 text-xs text-ink-muted">{v.readOnlyReason}</p>
      {/if}
      {#if v.isList}
        <details class="mt-1.5" open={flagged > 0}>
          <summary class="cursor-pointer text-xs text-ink-faint hover:text-ink">
            {t("envVars.entries", { count: v.entries.length })}
          </summary>
          {@render entryList(v)}
        </details>
      {:else}
        <p class="mt-1 select-text break-all font-mono text-[0.7rem] text-ink-muted">{v.value}</p>
      {/if}
    </div>
    {#if v.editable}
      <div class="flex shrink-0 flex-col items-end gap-1.5">
        <button type="button" class="btn-secondary px-3! py-1! text-xs!" disabled={busy} onclick={() => startEdit(v)}>
          {t("envVars.edit")}
        </button>
        <button type="button" class="btn-outline px-3! py-1! text-xs!" disabled={busy} onclick={() => reviewDelete(v)}>
          {t("envVars.delete")}
        </button>
        {#if undoable.has(v.name.toUpperCase())}
          <button type="button" class="btn-outline px-3! py-1! text-xs!" disabled={busy} onclick={() => undo(v.name)}>
            {t("envVars.undo")}
          </button>
        {/if}
      </div>
    {/if}
  </li>
{/snippet}

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(manifest.id, manifest.name) : MODULE_ID}
    info={[manifest ? tModuleDescription(manifest.id, manifest.description) : "", t("envVars.intro")]
      .filter(Boolean)
      .join(" ")}
  />

  {#if error}
    <p role="alert" class="rounded-xl border border-risk-danger/30 bg-surface-1 p-4 text-sm text-risk-danger">
      {error}
    </p>
  {/if}

  {#if status === "loading" && !listing}
    <LoadingNote text={t("envVars.loading")} />
  {:else if status === "error" && !listing}
    <div class="flex flex-col items-start gap-3 rounded-xl border border-surface-3 bg-surface-1 p-5">
      <p class="text-sm">{t("envVars.loadError")}</p>
      <button type="button" class="btn-secondary px-3! py-1.5!" onclick={() => load()}>
        {t("dashboard.retry")}
      </button>
    </div>
  {:else if listing}
    <div class="flex flex-wrap gap-2">
      <button type="button" class="btn-primary px-3! py-1! text-xs!" disabled={busy} onclick={startAdd}>
        {t("envVars.add")}
      </button>
      <button type="button" class="btn-outline px-2.5! py-1! text-xs!" disabled={busy} onclick={() => load()}>
        {t("envVars.refresh")}
      </button>
    </div>

    {#if report}
      <p class="rounded-xl bg-surface-2/60 p-4 text-sm text-ink-muted">{report}</p>
    {/if}

    {#if pending}
      <section class="flex flex-col gap-3 rounded-xl border border-accent/40 bg-surface-1 p-5">
        <h2 class="text-sm font-semibold">
          {t(pending.change.new ? "envVars.reviewTitle" : "envVars.reviewDeleteTitle", { name: pending.change.name })}
        </h2>
        <p class="text-xs text-ink-muted">
          {pending.preview.before ? kindLabel(pending.preview.before.expand) : t("envVars.absent")}
          → {pending.preview.after ? kindLabel(pending.preview.after.expand) : t("envVars.absent")}
        </p>
        <pre class="max-h-80 overflow-auto rounded-lg bg-surface-0 p-3 font-mono text-xs leading-relaxed">{#each pending.preview.diff as row, index (index)}{#if row.op === "gap"}<span class="text-ink-faint">{t("envVars.gap", { lines: row.lines })}</span>
{:else if row.op === "added"}<span class="text-risk-safe">+ {row.text}</span>
{:else if row.op === "removed"}<span class="text-risk-danger">- {row.text}</span>
{:else}<span class="text-ink-muted">  {row.text}</span>
{/if}{/each}</pre>
        <p class="text-xs text-ink-muted">{t("envVars.reviewNote")}</p>
        <div class="flex gap-2">
          <button type="button" class="btn-primary px-3! py-1! text-xs!" disabled={busy} onclick={save}>
            {busy ? t("envVars.working") : t(pending.change.new ? "envVars.confirmSave" : "envVars.confirmDelete")}
          </button>
          <button type="button" class="btn-outline px-3! py-1! text-xs!" disabled={busy} onclick={() => (pending = null)}>
            {t("envVars.back")}
          </button>
        </div>
      </section>
    {:else if draft}
      <section class="flex flex-col gap-3 rounded-xl border border-surface-3 bg-surface-1 p-5">
        <h2 class="text-sm font-semibold">
          {draft.isNew ? t("envVars.addTitle") : t("envVars.editTitle", { name: draft.name })}
        </h2>
        {#if draft.isNew}
          <label class="flex flex-col gap-1 text-xs">
            {t("envVars.nameLabel")}
            <input class="rounded-lg border border-surface-3 bg-surface-0 px-2 py-1 text-ink font-mono" bind:value={draft.name} spellcheck="false" />
          </label>
        {/if}
        <label class="flex items-center gap-2 text-xs">
          <input type="checkbox" bind:checked={draft.expand} />
          {t("envVars.expandLabel")}
        </label>
        <label class="flex items-center gap-2 text-xs">
          <input type="checkbox" checked={draft.listMode} onchange={() => draft && toggleListMode(draft)} />
          {t("envVars.listModeLabel")}
        </label>

        {#if draft.listMode}
          <ol class="flex flex-col gap-1.5">
            {#each draft.rows as row, index (index)}
              <li class="flex flex-col gap-0.5">
                <div class="flex items-center gap-1.5">
                  <input
                    class="rounded-lg border border-surface-3 bg-surface-0 px-2 py-1 text-ink min-w-0 flex-1 font-mono text-xs"
                    value={row.text}
                    aria-label={t("envVars.entryLabel", { index: index + 1 })}
                    spellcheck="false"
                    oninput={(event) => draft && editRow(draft, index, event.currentTarget.value)}
                  />
                  <button
                    type="button"
                    class="btn-outline px-2! py-0.5! text-xs!"
                    aria-label={t("envVars.moveUp")}
                    disabled={index === 0}
                    onclick={() => draft && moveRow(draft, index, -1)}>↑</button
                  >
                  <button
                    type="button"
                    class="btn-outline px-2! py-0.5! text-xs!"
                    aria-label={t("envVars.moveDown")}
                    disabled={index === draft.rows.length - 1}
                    onclick={() => draft && moveRow(draft, index, 1)}>↓</button
                  >
                  <button
                    type="button"
                    class="btn-outline px-2! py-0.5! text-xs!"
                    aria-label={t("envVars.removeEntry")}
                    onclick={() => draft && (draft.rows = draft.rows.filter((_, i) => i !== index))}>✕</button
                  >
                </div>
                {#each row.flags as flag (flag.kind)}
                  <p class="text-[0.7rem] text-risk-warning">{t(`envVars.flag.${flag.kind}`)}: {flag.reason}</p>
                {/each}
              </li>
            {/each}
          </ol>
          <div>
            <button
              type="button"
              class="btn-outline px-2.5! py-1! text-xs!"
              onclick={() => draft && (draft.rows = [...draft.rows, { text: "", expanded: null, flags: [] }])}
            >
              {t("envVars.addEntry")}
            </button>
          </div>
        {:else}
          <label class="flex flex-col gap-1 text-xs">
            {t("envVars.valueLabel")}
            <textarea class="rounded-lg border border-surface-3 bg-surface-0 px-2 py-1 text-ink min-h-20 font-mono text-xs" bind:value={draft.value} spellcheck="false"></textarea>
          </label>
        {/if}

        <div class="flex gap-2">
          <button type="button" class="btn-primary px-3! py-1! text-xs!" disabled={busy} onclick={() => draft && reviewDraft(draft)}>
            {t("envVars.review")}
          </button>
          <button type="button" class="btn-outline px-3! py-1! text-xs!" disabled={busy} onclick={() => (draft = null)}>
            {t("envVars.cancel")}
          </button>
        </div>
      </section>
    {/if}

    {#if deletedUndo.length > 0}
      <section class="flex flex-col gap-2">
        <h2 class="text-sm font-semibold">{t("envVars.deletedTitle")}</h2>
        <ul class="flex flex-col gap-2">
          {#each deletedUndo as entry (entry.name)}
            <li class="flex items-center gap-3 rounded-xl border border-surface-2 bg-surface-1 p-3">
              <span class="flex-1 font-mono text-sm">{entry.name}</span>
              <button type="button" class="btn-secondary px-3! py-1! text-xs!" disabled={busy} onclick={() => undo(entry.name)}>
                {t("envVars.restore")}
              </button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <section class="flex flex-col gap-2">
      <h2 class="text-sm font-semibold">{t("envVars.userTitle")}</h2>
      {#if listing.user.length === 0}
        <EmptyState message={t("envVars.userEmpty")} />
      {:else}
        <ul class="flex flex-col gap-2">
          {#each listing.user as v (v.name)}
            {@render variable(v)}
          {/each}
        </ul>
      {/if}
    </section>

    <section class="flex flex-col gap-2">
      <h2 class="text-sm font-semibold">{t("envVars.systemTitle")}</h2>
      <p class="text-xs text-ink-muted">{t("envVars.systemNote")}</p>
      <ul class="flex flex-col gap-2">
        {#each listing.system as v (v.name)}
          {@render variable(v)}
        {/each}
      </ul>
    </section>
  {/if}
</div>
