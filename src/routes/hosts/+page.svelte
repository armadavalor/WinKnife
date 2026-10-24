<!--
  Hosts file editor (K-019).

  Every edit happens on a draft in this page. The backend parses and diffs the
  draft read-only; only "Save" writes, after a confirm, a backup and a UAC
  prompt. Backups are never deleted and each can be restored with one click.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import {
    hostsBackups,
    hostsFlushDns,
    hostsPreview,
    hostsRead,
    hostsRestore,
    hostsRevealBackup,
    hostsSave,
  } from "$lib/api";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import LoadingNote from "$lib/components/LoadingNote.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import {
    isApiError,
    type HostsBackup,
    type HostsDocument,
    type HostsLine,
    type HostsPreview,
    type HostsSaveOutcome,
  } from "$lib/types";

  const MODULE_ID = "hosts";
  const PREVIEW_DELAY_MS = 250;

  const manifest = $derived(moduleStore.find(MODULE_ID));

  let doc = $state<HostsDocument | null>(null);
  let draft = $state("");
  let preview = $state<HostsPreview | null>(null);
  let backups = $state<HostsBackup[]>([]);
  let tab = $state<"entries" | "text">("entries");
  let newIp = $state("");
  let newNames = $state("");
  let busy = $state(false);
  let confirmSave = $state(false);
  let restoreTarget = $state<HostsBackup | null>(null);
  let outcome = $state<HostsSaveOutcome | null>(null);
  let flushing = $state(false);
  let flushOutput = $state<string | null>(null);
  let error = $state<string | null>(null);

  const readOnly = $derived(doc?.readOnly ?? null);
  const editable = $derived(doc !== null && readOnly === null && !busy);
  const lines = $derived(preview?.lines ?? doc?.lines ?? []);
  const entries = $derived(lines.filter(isEntryLike));
  const problems = $derived(preview?.problems ?? []);
  const changed = $derived(preview?.changed ?? false);
  const canSave = $derived(editable && changed && problems.length === 0);

  function message(err: unknown): string {
    return isApiError(err) ? err.message : String(err);
  }

  function isEntryLike(line: HostsLine): boolean {
    const kind = line.kind;
    return kind.kind === "entry" || kind.kind === "invalid" || (kind.kind === "comment" && kind.disabled);
  }

  async function load(): Promise<void> {
    try {
      const [read, list] = await Promise.all([hostsRead(), hostsBackups()]);
      doc = read;
      draft = read.text;
      backups = list;
    } catch (err: unknown) {
      error = message(err);
    }
  }

  onMount(() => {
    void load();
  });

  // Re-parse and re-diff the draft shortly after each edit.
  $effect(() => {
    if (doc === null) return;
    const text = draft;
    const timer = setTimeout(() => {
      hostsPreview(text)
        .then((result) => {
          if (text === draft) preview = result;
        })
        .catch((err: unknown) => {
          error = message(err);
        });
    }, PREVIEW_DELAY_MS);
    return () => clearTimeout(timer);
  });

  /** Replace draft line `number` (1-based) with `next`, or drop it when null. */
  function editLine(number: number, next: string | null): void {
    const all = draft.split("\n");
    all.splice(number - 1, 1, ...(next === null ? [] : [next]));
    draft = all.join("\n");
  }

  function toggle(line: HostsLine): void {
    const enabled = line.kind.kind === "entry";
    editLine(line.number, enabled ? `# ${line.text}` : line.text.replace(/^\s*#\s?/, ""));
  }

  function addEntry(): void {
    const entry = `${newIp.trim()} ${newNames.trim().split(/\s+/).join(" ")}`;
    const all = draft.split("\n");
    // Keep the file's final newline after the new entry.
    if (all.length > 0 && all[all.length - 1] === "") {
      all.splice(all.length - 1, 0, entry);
    } else {
      all.push(entry);
    }
    draft = all.join("\n");
    newIp = "";
    newNames = "";
  }

  async function finish(run: () => Promise<HostsSaveOutcome>): Promise<void> {
    busy = true;
    outcome = null;
    error = null;
    try {
      outcome = await run();
    } catch (err: unknown) {
      error = message(err);
    } finally {
      busy = false;
    }
    // Whatever happened, show the file as it now is on disk.
    if (outcome && outcome.state !== "declined" && outcome.state !== "invalid") {
      await load();
    } else {
      backups = await hostsBackups().catch(() => backups);
    }
  }

  function save(): void {
    confirmSave = false;
    void finish(() => hostsSave(draft));
  }

  function restore(): void {
    const target = restoreTarget;
    restoreTarget = null;
    if (target) void finish(() => hostsRestore(target.name));
  }

  async function reveal(name: string): Promise<void> {
    try {
      await hostsRevealBackup(name);
    } catch (err: unknown) {
      error = message(err);
    }
  }

  async function flush(): Promise<void> {
    flushing = true;
    flushOutput = null;
    error = null;
    try {
      flushOutput = await hostsFlushDns();
    } catch (err: unknown) {
      error = message(err);
    } finally {
      flushing = false;
    }
  }

  function outcomeText(result: HostsSaveOutcome): string {
    if (result.state === "failed") {
      return t("hosts.outcome.failed", { code: result.code });
    }
    return t(`hosts.outcome.${result.state}`, { backup: result.backup ?? "" });
  }

  function outcomeTone(result: HostsSaveOutcome): string {
    switch (result.state) {
      case "written":
        return "border-risk-safe/30 bg-surface-1 text-risk-safe";
      case "verifyFailed":
      case "failed":
      case "timedOut":
        return "border-risk-danger/30 bg-surface-1 text-risk-danger";
      default:
        return "border-risk-review/30 bg-surface-1 text-risk-review";
    }
  }

  function formatDate(iso: string): string {
    return new Date(iso).toLocaleString();
  }
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(manifest.id, manifest.name) : MODULE_ID}
    info={manifest ? tModuleDescription(manifest.id, manifest.description) : undefined}
  />

  {#if error}
    <p class="rounded-xl border border-risk-danger/30 bg-surface-1 p-4 text-sm text-risk-danger">
      {error}
    </p>
  {/if}

  {#if doc === null}
    {#if !error}<LoadingNote text={t("hosts.loading")} />{/if}
  {:else}
    <section class="flex flex-col gap-1 rounded-xl border border-surface-2 bg-surface-1 p-6">
      <span class="text-xs font-medium uppercase tracking-wider text-ink-faint">
        {t("hosts.fileLabel")}
      </span>
      <p class="select-text break-all font-mono text-sm">{doc.path}</p>
      <p class="text-xs text-ink-muted">
        {doc.exists
          ? t("hosts.format", { encoding: doc.encoding, lineEnding: doc.lineEnding })
          : t("hosts.missing")}
      </p>
      {#if readOnly}
        <p class="mt-2 text-sm text-risk-review">{t("hosts.readOnly", { reason: readOnly })}</p>
      {/if}
    </section>

    <section class="flex flex-col gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6">
      <div class="flex gap-2" role="tablist">
        {#each ["entries", "text"] as const as id (id)}
          <button
            type="button"
            role="tab"
            aria-selected={tab === id}
            class={tab === id ? "btn-primary px-3! py-1.5!" : "btn-outline px-3! py-1.5!"}
            onclick={() => (tab = id)}
          >
            {t(`hosts.tab.${id}`)}
          </button>
        {/each}
      </div>

      {#if tab === "entries"}
        {#if entries.length === 0}
          <p class="text-sm text-ink-faint">{t("hosts.entriesEmpty")}</p>
        {:else}
          <ul class="flex flex-col divide-y divide-surface-2">
            {#each entries as line (line.number)}
              <li class="flex items-center gap-3 py-2">
                <span class="w-8 shrink-0 text-right font-mono text-xs text-ink-faint">{line.number}</span>
                <div class="flex min-w-0 flex-1 flex-col">
                  {#if line.kind.kind === "entry"}
                    <span class="truncate font-mono text-sm">
                      {line.kind.ip}
                      <span class="text-ink-muted">{line.kind.hostnames.join(" ")}</span>
                    </span>
                  {:else if line.kind.kind === "invalid"}
                    <span class="truncate font-mono text-sm">{line.text}</span>
                    <span class="text-xs text-risk-danger">
                      {t("hosts.invalid", { line: line.number, reason: line.kind.reason })}
                    </span>
                  {:else}
                    <span class="truncate font-mono text-sm text-ink-faint line-through">{line.text}</span>
                  {/if}
                </div>
                {#if line.kind.kind !== "invalid"}
                  <span class="text-xs {line.kind.kind === 'entry' ? 'text-risk-safe' : 'text-ink-faint'}">
                    {line.kind.kind === "entry" ? t("hosts.enabled") : t("hosts.disabled")}
                  </span>
                  <button
                    type="button"
                    class="btn-outline px-3! py-1.5!"
                    disabled={!editable}
                    onclick={() => toggle(line)}
                  >
                    {line.kind.kind === "entry" ? t("hosts.turnOff") : t("hosts.turnOn")}
                  </button>
                {/if}
                <button
                  type="button"
                  class="btn-outline px-3! py-1.5!"
                  disabled={!editable}
                  onclick={() => editLine(line.number, null)}
                >
                  {t("hosts.remove")}
                </button>
              </li>
            {/each}
          </ul>
        {/if}

        <form
          class="flex flex-wrap items-end gap-2"
          onsubmit={(event) => {
            event.preventDefault();
            addEntry();
          }}
        >
          <span class="w-full text-xs font-medium uppercase tracking-wider text-ink-faint">
            {t("hosts.addTitle")}
          </span>
          <input
            type="text"
            aria-label={t("hosts.addIp")}
            placeholder={t("hosts.addIp")}
            bind:value={newIp}
            disabled={!editable}
            class="w-40 rounded-lg border border-surface-3 bg-surface-0 px-3 py-1.5 font-mono text-sm text-ink"
          />
          <input
            type="text"
            aria-label={t("hosts.addNames")}
            placeholder={t("hosts.addNames")}
            bind:value={newNames}
            disabled={!editable}
            class="min-w-48 flex-1 rounded-lg border border-surface-3 bg-surface-0 px-3 py-1.5 font-mono text-sm text-ink"
          />
          <button
            type="submit"
            class="btn-outline px-3! py-1.5!"
            disabled={!editable || newIp.trim() === "" || newNames.trim() === ""}
          >
            {t("hosts.add")}
          </button>
        </form>
      {:else}
        <label class="flex flex-col gap-2">
          <span class="text-xs font-medium uppercase tracking-wider text-ink-faint">
            {t("hosts.textLabel")}
          </span>
          <textarea
            bind:value={draft}
            readonly={!editable}
            spellcheck="false"
            rows="18"
            class="rounded-lg border border-surface-3 bg-surface-0 p-3 font-mono text-sm text-ink"
          ></textarea>
        </label>
      {/if}
    </section>

    <section class="flex flex-col gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6">
      <span class="text-xs font-medium uppercase tracking-wider text-ink-faint">
        {t("hosts.reviewTitle")}
      </span>

      {#if problems.length > 0}
        <div class="flex flex-col gap-1 text-sm text-risk-danger" role="alert">
          <p>{t("hosts.problems")}</p>
          <ul class="list-disc pl-5">
            {#each problems as problem, index (index)}
              <li>
                {problem.line === null
                  ? problem.reason
                  : t("hosts.invalid", { line: problem.line, reason: problem.reason })}
              </li>
            {/each}
          </ul>
        </div>
      {/if}

      {#if changed && preview}
        <pre class="max-h-80 overflow-auto rounded-lg bg-surface-0 p-3 font-mono text-xs leading-relaxed">{#each preview.diff as row, index (index)}{#if row.op === "gap"}<span class="text-ink-faint">{t("hosts.gap", { lines: row.lines })}</span>
{:else if row.op === "added"}<span class="text-risk-safe">+ {row.text}</span>
{:else if row.op === "removed"}<span class="text-risk-danger">- {row.text}</span>
{:else}<span class="text-ink-muted">  {row.text}</span>
{/if}{/each}</pre>
      {:else}
        <p class="text-sm text-ink-faint">{t("hosts.noChanges")}</p>
      {/if}

      <div class="flex items-center gap-2">
        <button type="button" class="btn-primary" disabled={!canSave} onclick={() => (confirmSave = true)}>
          {busy ? t("hosts.saving") : t("hosts.save")}
        </button>
        <button
          type="button"
          class="btn-outline px-3! py-1.5!"
          disabled={!changed || busy}
          onclick={() => (draft = doc?.text ?? draft)}
        >
          {t("hosts.discard")}
        </button>
      </div>

      <details class="group">
        <summary class="cursor-pointer text-xs text-ink-faint hover:text-ink">
          {t("hosts.uacSummary")}
        </summary>
        <p class="mt-2 rounded-xl bg-surface-2/60 p-4 text-sm leading-relaxed text-ink-muted">
          {t("hosts.uacNotice")}
        </p>
      </details>

      {#if outcome}
        <p class="rounded-xl border p-4 text-sm leading-relaxed {outcomeTone(outcome)}" role="status">
          {outcomeText(outcome)}
        </p>
      {/if}
    </section>

    <section class="flex flex-col gap-3 rounded-xl border border-surface-2 bg-surface-1 p-6">
      <span class="text-xs font-medium uppercase tracking-wider text-ink-faint">
        {t("hosts.backupsTitle")}
      </span>
      {#if backups.length === 0}
        <p class="text-sm text-ink-faint">{t("hosts.backupsEmpty")}</p>
      {:else}
        <ul class="flex flex-col divide-y divide-surface-2">
          {#each backups as backup (backup.name)}
            <li class="flex items-center gap-3 py-2">
              <div class="flex min-w-0 flex-1 flex-col">
                <span class="text-sm">{formatDate(backup.takenAt)}</span>
                <span class="truncate font-mono text-xs text-ink-faint">{backup.name}</span>
              </div>
              <button
                type="button"
                class="btn-outline px-3! py-1.5!"
                disabled={readOnly !== null || busy}
                onclick={() => (restoreTarget = backup)}
              >
                {t("hosts.restore")}
              </button>
              <button type="button" class="btn-outline px-3! py-1.5!" onclick={() => reveal(backup.name)}>
                {t("hosts.reveal")}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section class="flex flex-col items-start gap-3 rounded-xl border border-surface-2 bg-surface-1 p-6">
      <span class="text-xs font-medium uppercase tracking-wider text-ink-faint">
        {t("hosts.flushTitle")}
      </span>
      <p class="text-sm leading-relaxed text-ink-muted">{t("hosts.flushHint")}</p>
      <code class="select-text rounded bg-surface-0 px-2 py-1 font-mono text-xs">{doc.flushDnsCommand}</code>
      <button type="button" class="btn-outline px-3! py-1.5!" disabled={flushing} onclick={flush}>
        {flushing ? t("hosts.flushing") : t("hosts.flush")}
      </button>
      {#if flushOutput !== null}
        <pre class="w-full overflow-auto rounded-lg bg-surface-0 p-3 font-mono text-xs" role="status">{flushOutput}</pre>
      {/if}
    </section>
  {/if}
</div>

<ConfirmDialog
  open={confirmSave}
  title={t("hosts.confirmTitle")}
  message={t("hosts.confirmBody")}
  note={t("hosts.confirmNote")}
  detail={doc?.path}
  confirmLabel={t("hosts.confirmSave")}
  onconfirm={save}
  oncancel={() => (confirmSave = false)}
/>

<ConfirmDialog
  open={restoreTarget !== null}
  title={t("hosts.restoreTitle")}
  message={t("hosts.restoreBody", { date: restoreTarget ? formatDate(restoreTarget.takenAt) : "" })}
  detail={restoreTarget?.name}
  confirmLabel={t("hosts.restore")}
  onconfirm={restore}
  oncancel={() => (restoreTarget = null)}
/>
