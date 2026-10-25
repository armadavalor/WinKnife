<!--
  Repair kit (C6).

  A page of buttons for Windows' own SFC and DISM. Nothing runs until a click;
  each tool shows its exact command line, and the two that replace system files
  (sfc, RestoreHealth) confirm first. Unelevated runs come back `needs_admin`,
  which raises the app's "Restart as administrator" banner (K-021).
-->
<script lang="ts">
  import { onMount } from "svelte";

  import { onRepairOutput, repairCancel, repairRun, repairTools, revealInExplorer } from "$lib/api";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import LoadingNote from "$lib/components/LoadingNote.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { isApiError, type RepairOutcome, type RepairTool, type RepairToolInfo } from "$lib/types";

  const MODULE_ID = "repair";
  /** Lines kept on screen; the full output is in the saved log. */
  const MAX_LINES = 500;

  const manifest = $derived(moduleStore.find(MODULE_ID));

  let tools = $state<RepairToolInfo[] | null>(null);
  let running = $state<RepairTool | null>(null);
  let confirmTool = $state<RepairToolInfo | null>(null);
  let lines = $state<string[]>([]);
  let lastIsProgress = false;
  let outcome = $state<RepairOutcome | null>(null);
  let error = $state<string | null>(null);

  function message(err: unknown): string {
    return isApiError(err) ? err.message : String(err);
  }

  onMount(() => {
    repairTools()
      .then((list) => (tools = list))
      .catch((err: unknown) => (error = message(err)));
    const unlisten = onRepairOutput((line) => {
      const next = lastIsProgress ? lines.slice(0, -1) : lines.slice();
      next.push(line.text);
      lines = next.slice(-MAX_LINES);
      lastIsProgress = line.progress;
    });
    return () => void unlisten.then((stop) => stop());
  });

  function request(info: RepairToolInfo): void {
    if (info.needsConfirm) {
      confirmTool = info;
    } else {
      void run(info.tool);
    }
  }

  async function run(tool: RepairTool): Promise<void> {
    confirmTool = null;
    running = tool;
    lines = [];
    lastIsProgress = false;
    outcome = null;
    error = null;
    try {
      outcome = await repairRun(tool);
    } catch (err: unknown) {
      error = message(err);
    } finally {
      running = null;
    }
  }

  async function cancel(): Promise<void> {
    try {
      await repairCancel();
    } catch (err: unknown) {
      error = message(err);
    }
  }

  async function openLog(path: string): Promise<void> {
    try {
      await revealInExplorer(path);
    } catch (err: unknown) {
      error = message(err);
    }
  }

  function verdictTone(result: RepairOutcome): string {
    switch (result.verdict) {
      case "no-problems":
      case "repaired":
      case "cleanup-not-needed":
        return "border-risk-safe/30 text-risk-safe";
      case "could-not-repair":
      case "failed":
        return "border-risk-danger/30 text-risk-danger";
      default:
        return "border-risk-review/30 text-risk-review";
    }
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

  {#if tools === null}
    {#if !error}<LoadingNote text={t("repair.loading")} />{/if}
  {:else}
    <section class="flex flex-col gap-3 rounded-xl border border-surface-2 bg-surface-1 p-6">
      <p class="text-sm leading-relaxed text-ink-muted">{t("repair.intro")}</p>
      <ul class="flex flex-col divide-y divide-surface-2">
        {#each tools as info (info.tool)}
          <li class="flex items-center gap-3 py-3">
            <div class="flex min-w-0 flex-1 flex-col gap-1">
              <span class="text-sm font-medium">{t(`repair.tool.${info.tool}`)}</span>
              <span class="text-xs leading-relaxed text-ink-muted">{t(`repair.tool.${info.tool}.hint`)}</span>
              <code class="select-text self-start rounded bg-surface-0 px-2 py-1 font-mono text-xs">{info.command}</code>
            </div>
            <button
              type="button"
              class="btn-outline px-3! py-1.5!"
              disabled={running !== null}
              onclick={() => request(info)}
            >
              {running === info.tool ? t("repair.running") : t("repair.run")}
            </button>
          </li>
        {/each}
      </ul>
    </section>

    {#if running !== null || lines.length > 0 || outcome}
      <section class="flex flex-col gap-3 rounded-xl border border-surface-2 bg-surface-1 p-6">
        <div class="flex items-center gap-2">
          <span class="flex-1 text-xs font-medium uppercase tracking-wider text-ink-faint">
            {t("repair.outputLabel")}
          </span>
          {#if running !== null}
            <button type="button" class="btn-outline px-3! py-1.5!" onclick={cancel}>
              {t("repair.cancel")}
            </button>
          {/if}
        </div>
        <pre
          class="max-h-96 overflow-auto whitespace-pre-wrap rounded-lg bg-surface-0 p-3 font-mono text-xs leading-relaxed"
          aria-live="off">{lines.join("\n")}</pre>
        {#if outcome}
          <div class="flex flex-wrap items-center gap-3 rounded-xl border bg-surface-1 p-4 {verdictTone(outcome)}">
            <p class="flex-1 text-sm" role="status">
              {t(`repair.verdict.${outcome.verdict}`, { code: outcome.exitCode ?? "" })}
            </p>
            {#if outcome.logPath}
              {@const logPath = outcome.logPath}
              <button type="button" class="btn-outline px-3! py-1.5!" onclick={() => openLog(logPath)}>
                {t("repair.openLog")}
              </button>
            {/if}
          </div>
        {/if}
      </section>
    {/if}
  {/if}
</div>

<ConfirmDialog
  open={confirmTool !== null}
  title={t("repair.confirmTitle", { name: confirmTool ? t(`repair.tool.${confirmTool.tool}`) : "" })}
  message={confirmTool?.tool === "sfc" ? t("repair.confirm.sfc") : t("repair.confirm.restore-health")}
  detail={confirmTool?.command}
  confirmLabel={t("repair.confirmRun")}
  onconfirm={() => {
    if (confirmTool) void run(confirmTool.tool);
  }}
  oncancel={() => (confirmTool = null)}
/>
