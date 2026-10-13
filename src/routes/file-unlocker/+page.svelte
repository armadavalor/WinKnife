<!--
  File Unlocker: which program is holding this file open?

  Checking is read-only (the Restart Manager only reports). The single action
  is "Ask to close", offered only for window and console apps, behind a
  confirm dialog, and it is the same polite request Windows sends at sign-out:
  the program may ask to save, or decline. Rust re-checks every refusal rule
  (services, Explorer, critical, System, WinKnife itself, PID reuse), so the
  button being hidden is a courtesy, not the safeguard.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import {
    onFileDrop,
    pickFiles,
    pickFolder,
    revealInExplorer,
    unlockerQuery,
    unlockerRequestClose,
  } from "$lib/api";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import InfoTip from "$lib/components/InfoTip.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { t, tDynamic, tModuleName } from "$lib/i18n";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import type { CloseOutcome, LockRow, UnlockerReport } from "$lib/types";

  const MODULE_ID = "file-unlocker";
  const manifest = $derived(moduleStore.find(MODULE_ID));

  type Status = "idle" | "checking" | "done" | "error";

  let paths = $state<string[]>([]);
  let typedPath = $state("");
  let status = $state<Status>("idle");
  let report = $state<UnlockerReport | null>(null);
  let errorText = $state<string | null>(null);
  let hovering = $state(false);
  let pending = $state<LockRow | null>(null);
  let closingPid = $state<number | null>(null);
  let notice = $state<string | null>(null);

  onMount(() => {
    const unlisten = onFileDrop({
      onHover: (value) => (hovering = value),
      onDrop: (dropped) => void check(dropped),
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  });

  function describeError(err: unknown): string {
    if (err && typeof err === "object" && "message" in err) {
      return String((err as { message: unknown }).message);
    }
    return String(err);
  }

  async function check(next: string[]): Promise<void> {
    if (next.length === 0) {
      return;
    }
    paths = next;
    status = "checking";
    errorText = null;
    try {
      report = await unlockerQuery(next);
      status = "done";
    } catch (err) {
      errorText = describeError(err);
      status = "error";
    }
  }

  async function pick(which: "files" | "folder"): Promise<void> {
    try {
      const picked = which === "files" ? await pickFiles() : await pickFolder();
      await check(picked);
    } catch {
      errorText = t("unlocker.pickError");
      status = "error";
    }
  }

  function addTyped(event: SubmitEvent): void {
    event.preventDefault();
    const value = typedPath.trim();
    if (value.length > 0) {
      typedPath = "";
      void check([value]);
    }
  }

  function clear(): void {
    paths = [];
    report = null;
    status = "idle";
    errorText = null;
    notice = null;
  }

  function holdsText(row: LockRow): string {
    return row.attributed
      ? t("unlocker.holdsSome", { count: row.held_count })
      : t("unlocker.holdsUnattributed");
  }

  function reasonText(row: LockRow): string {
    const policy = tDynamic(`unlocker.reason.${row.reason_code}`, "");
    return policy ? `${holdsText(row)} ${policy}` : row.reason;
  }

  function outcomeText(app: string, outcome: CloseOutcome): string {
    if (outcome.status === "refused") {
      return t("unlocker.outcome.refused", { app, reason: outcome.reason });
    }
    return tDynamic(`unlocker.outcome.${outcome.status}`, outcome.status, { app });
  }

  async function confirmClose(): Promise<void> {
    const row = pending;
    pending = null;
    if (!row) {
      return;
    }
    closingPid = row.pid;
    notice = null;
    try {
      const result = await unlockerRequestClose(row.pid, row.start_time);
      notice = outcomeText(row.app_name, result.outcome);
    } catch (err) {
      notice = `${t("unlocker.closeError", { app: row.app_name })} ${describeError(err)}`;
    } finally {
      closingPid = null;
    }
    await check(paths);
  }
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(MODULE_ID, manifest.name) : t("unlocker.title")}
    info={t("unlocker.subtitle")}
  />

  <section
    class="flex flex-col gap-3 rounded-xl border-2 border-dashed p-5 transition-colors {hovering
      ? 'border-accent bg-accent-soft'
      : 'border-surface-3 bg-surface-1'}"
    aria-label={t("unlocker.dropHint")}
  >
    <div class="flex flex-wrap items-center gap-2">
      <button
        type="button"
        class="rounded-lg bg-surface-3 px-3 py-1.5 text-sm transition-colors hover:bg-accent-soft"
        disabled={status === "checking"}
        onclick={() => void pick("files")}
      >
        {t("unlocker.pickFiles")}
      </button>
      <button
        type="button"
        class="rounded-lg bg-surface-3 px-3 py-1.5 text-sm transition-colors hover:bg-accent-soft"
        disabled={status === "checking"}
        onclick={() => void pick("folder")}
      >
        {t("unlocker.pickFolder")}
      </button>
      <form class="flex min-w-64 flex-1 items-center gap-2" onsubmit={addTyped}>
        <label class="sr-only" for="unlocker-path">{t("unlocker.pathLabel")}</label>
        <input
          id="unlocker-path"
          class="min-w-0 flex-1 rounded-lg border border-surface-3 bg-surface-0 px-3 py-1.5 font-mono text-xs"
          placeholder={t("unlocker.pathPlaceholder")}
          bind:value={typedPath}
          disabled={status === "checking"}
        />
        <button
          type="submit"
          class="rounded-lg border border-surface-3 px-3 py-1.5 text-sm transition-colors hover:bg-surface-2"
          disabled={status === "checking" || typedPath.trim().length === 0}
        >
          {t("unlocker.addPath")}
        </button>
      </form>
      <InfoTip text={t("unlocker.dropHint")} />
    </div>
    {#if hovering}
      <p class="text-xs text-ink-faint">{t("unlocker.dropActive")}</p>
    {/if}
  </section>

  {#if paths.length > 0}
    <div class="flex flex-wrap items-center gap-3 text-sm text-ink-muted">
      <span>{t("unlocker.picked", { count: paths.length })}</span>
      <button
        type="button"
        class="rounded-lg border border-surface-3 px-2.5 py-1 text-xs transition-colors hover:bg-surface-2 hover:text-ink"
        disabled={status === "checking"}
        onclick={() => void check(paths)}
      >
        {t("unlocker.recheck")}
      </button>
      <button
        type="button"
        class="rounded-lg border border-surface-3 px-2.5 py-1 text-xs transition-colors hover:bg-surface-2 hover:text-ink"
        disabled={status === "checking"}
        onclick={clear}
      >
        {t("unlocker.clear")}
      </button>
    </div>
  {/if}

  {#if notice}
    <p class="rounded-lg bg-surface-2 p-3 text-sm" role="status">{notice}</p>
  {/if}

  {#if status === "checking"}
    <p class="text-sm text-ink-faint">{t("unlocker.checking")}</p>
  {:else if status === "error"}
    <div class="rounded-xl border border-risk-danger/30 p-4 text-sm text-risk-danger" role="alert">
      <p>{t("unlocker.queryError")}</p>
      {#if errorText}<p class="mt-1 font-mono text-xs">{errorText}</p>{/if}
    </div>
  {/if}

  {#if report && status !== "checking"}
    <p class="text-xs text-ink-faint">{t("unlocker.filesChecked", { count: report.files_checked })}</p>

    {#if report.capped}
      <p class="rounded-lg border border-risk-warning/40 p-3 text-xs leading-relaxed text-risk-warning">
        {t("unlocker.capNote", { cap: report.cap.toLocaleString() })}
      </p>
    {/if}
    {#if report.unreadable > 0}
      <p class="text-xs text-ink-muted">{t("unlocker.unreadable", { count: report.unreadable })}</p>
    {/if}
    {#if report.skipped.length > 0}
      <div class="rounded-lg bg-surface-2 p-3 text-xs leading-relaxed text-ink-muted">
        <p class="font-medium">{t("unlocker.skippedTitle")}</p>
        <ul class="mt-1 flex flex-col gap-0.5">
          {#each report.skipped as skipped (skipped.path)}
            <li><span class="font-mono">{skipped.path}</span> — {skipped.reason}</li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if report.rows.length === 0}
      <EmptyState message={t("unlocker.emptyTitle")}>
        {#snippet action()}
          <InfoTip text={t("unlocker.emptyHint")} />
        {/snippet}
      </EmptyState>
    {:else}
      <ul class="flex flex-col gap-2">
        {#each report.rows as row (row.pid + ":" + row.start_time)}
          <li
            class="flex flex-wrap items-start gap-4 rounded-xl border border-surface-2 bg-surface-1 p-4 {row.can_close
              ? ''
              : 'opacity-80'}"
          >
            <div class="min-w-0 flex-1">
              <p class="flex flex-wrap items-center gap-2">
                <span class="font-medium">{row.app_name}</span>
                <span class="rounded-md bg-surface-3 px-1.5 py-0.5 text-[0.65rem] text-ink-muted">
                  {t(`unlocker.kind.${row.kind}`)}
                </span>
                <span class="text-[0.7rem] text-ink-faint">{t("unlocker.pid", { pid: row.pid })}</span>
                {#if row.service_name}
                  <span class="text-[0.7rem] text-ink-faint">
                    {t("unlocker.service", { name: row.service_name })}
                  </span>
                {/if}
              </p>
              {#if row.exe_path}
                <p class="mt-1 truncate font-mono text-[0.7rem] text-ink-faint" title={row.exe_path}>
                  {row.exe_path}
                </p>
              {/if}
              <p class="mt-1.5 text-xs leading-relaxed text-ink-muted" title={row.reason}>
                {reasonText(row)}
              </p>
              {#if row.held_files.length > 0}
                <ul class="mt-1.5 flex flex-col gap-0.5">
                  {#each row.held_files as file (file)}
                    <li class="flex items-center gap-2 text-[0.7rem]">
                      <span class="truncate font-mono text-ink-faint" title={file}>{file}</span>
                      <button
                        type="button"
                        class="shrink-0 text-accent underline-offset-2 hover:underline"
                        onclick={() => void revealInExplorer(file)}
                      >
                        {t("unlocker.openLocation")}
                      </button>
                    </li>
                  {/each}
                </ul>
                {#if row.held_count > row.held_files.length}
                  <p class="mt-0.5 text-[0.7rem] text-ink-faint">
                    {t("unlocker.heldMore", { count: row.held_count - row.held_files.length })}
                  </p>
                {/if}
              {/if}
            </div>
            {#if row.can_close}
              <button
                type="button"
                class="shrink-0 rounded-lg bg-surface-3 px-3 py-1 text-xs font-medium transition-colors hover:bg-accent-soft"
                disabled={closingPid !== null}
                onclick={() => (pending = row)}
              >
                {closingPid === row.pid ? t("unlocker.asking") : t("unlocker.askClose")}
              </button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>

<ConfirmDialog
  open={pending !== null}
  title={t("unlocker.confirmTitle", { app: pending?.app_name ?? "" })}
  message={t("unlocker.confirmMessage", { app: pending?.app_name ?? "" })}
  note={t("unlocker.confirmNote")}
  detail={pending?.exe_path ?? undefined}
  confirmLabel={t("unlocker.askClose")}
  onconfirm={() => void confirmClose()}
  oncancel={() => (pending = null)}
/>
