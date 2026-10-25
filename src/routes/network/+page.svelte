<!--
  Network (C14): which program listens on, or is connected to, which port.
  Read-only. The one action is flushing the DNS cache, behind a confirm.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import { hostsFlushDns, networkList } from "$lib/api";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import ListToolbar from "$lib/components/ListToolbar.svelte";
  import LoadingNote from "$lib/components/LoadingNote.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { matchesFilter } from "$lib/listTools";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { isApiError, type PortRow } from "$lib/types";

  const MODULE_ID = "network";
  const COPIED_MS = 1500;

  const manifest = $derived(moduleStore.find(MODULE_ID));

  type KindFilter = "all" | PortRow["kind"];

  let rows = $state<PortRow[]>([]);
  let loaded = $state(false);
  let refreshing = $state(false);
  let error = $state<string | null>(null);
  let filter = $state("");
  let kind = $state<KindFilter>("all");
  let copiedKey = $state<string | null>(null);
  let confirmFlush = $state(false);
  let flushing = $state(false);
  let flushReport = $state<string | null>(null);

  const shown = $derived(
    rows.filter(
      (row) =>
        (kind === "all" || row.kind === kind) &&
        matchesFilter(
          filter,
          row.processName,
          row.exePath,
          row.localAddress,
          row.remoteAddress,
          String(row.localPort),
          String(row.remotePort ?? ""),
          String(row.pid),
        ),
    ),
  );

  function message(err: unknown): string {
    return isApiError(err) ? err.message : String(err);
  }

  async function load(): Promise<void> {
    refreshing = true;
    error = null;
    try {
      rows = await networkList();
      loaded = true;
    } catch (err: unknown) {
      error = message(err);
    } finally {
      refreshing = false;
    }
  }

  onMount(() => {
    void load();
  });

  const keyOf = (row: PortRow): string =>
    [row.protocol, row.localAddress, row.localPort, row.remoteAddress, row.remotePort, row.pid].join("|");

  function rowText(row: PortRow): string {
    const remote = row.remoteAddress ? ` -> ${row.remoteAddress}:${row.remotePort}` : "";
    const exe = row.exePath ? ` ${row.exePath}` : "";
    return `${row.protocol} ${row.localAddress}:${row.localPort}${remote} ${row.processName} (PID ${row.pid})${exe}`;
  }

  async function copy(row: PortRow): Promise<void> {
    try {
      await navigator.clipboard.writeText(rowText(row));
      copiedKey = keyOf(row);
      setTimeout(() => (copiedKey = null), COPIED_MS);
    } catch (err: unknown) {
      error = message(err);
    }
  }

  async function flush(): Promise<void> {
    confirmFlush = false;
    flushing = true;
    flushReport = null;
    try {
      await hostsFlushDns();
      flushReport = t("network.flushDone");
    } catch (err: unknown) {
      flushReport = t("network.flushFailed", { error: message(err) });
    } finally {
      flushing = false;
    }
  }
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(manifest.id, manifest.name) : MODULE_ID}
    info={[manifest ? tModuleDescription(manifest.id, manifest.description) : "", t("network.intro")]
      .filter(Boolean)
      .join(" ")}
  />

  {#if error}
    <p class="rounded-xl border border-risk-danger/30 bg-surface-1 p-4 text-sm text-risk-danger">
      {t("network.loadError")} {error}
    </p>
  {/if}

  {#if !loaded}
    {#if !error}<LoadingNote text={t("network.loading")} />{/if}
  {:else}
    <div class="flex flex-col gap-3">
      <div class="flex flex-wrap items-center gap-2">
        <div class="min-w-0 flex-1">
          <ListToolbar bind:filter shown={shown.length} total={rows.length} />
        </div>
        <select
          class="rounded-lg border border-surface-3 bg-surface-0 px-2 py-1.5 text-sm"
          aria-label={t("network.kind.all")}
          bind:value={kind}
        >
          <option value="all">{t("network.kind.all")}</option>
          <option value="listening">{t("network.kind.listening")}</option>
          <option value="connection">{t("network.kind.connection")}</option>
        </select>
        <button
          type="button"
          class="btn-outline px-3! py-1.5! text-xs"
          disabled={refreshing}
          onclick={load}
        >
          {refreshing ? t("network.refreshing") : t("network.refresh")}
        </button>
      </div>

      {#if rows.length === 0}
        <EmptyState message={t("network.empty")} />
      {:else if shown.length === 0}
        <EmptyState message={t("list.noMatch")} />
      {:else}
        <ul class="flex flex-col gap-2">
          {#each shown as row (keyOf(row))}
            <li class="flex flex-wrap items-start gap-4 rounded-xl border border-surface-2 bg-surface-1 p-4">
              <div class="min-w-0 flex-1">
                <p class="flex flex-wrap items-center gap-2">
                  <span class="font-medium">{row.processName}</span>
                  <span class="rounded-md bg-surface-2 px-1.5 py-0.5 text-[0.65rem] text-ink-faint">
                    {row.protocol}{row.state ? ` · ${row.state}` : ""}
                  </span>
                  <span class="rounded-md bg-surface-2 px-1.5 py-0.5 text-[0.65rem] text-ink-faint">
                    {t(`network.reach.${row.reach}`)}
                  </span>
                  <span class="text-[0.7rem] tabular-nums text-ink-faint">
                    {t("network.pid", { pid: row.pid })}
                  </span>
                </p>
                <p class="mt-1 select-text font-mono text-xs text-ink-muted">
                  {row.localAddress}:{row.localPort}
                  {#if row.remoteAddress}
                    {t("network.remote", { address: row.remoteAddress, port: row.remotePort ?? 0 })}
                  {/if}
                </p>
                {#if row.exePath}
                  <p class="mt-1 select-text break-all font-mono text-[0.7rem] text-ink-faint">
                    {row.exePath}
                  </p>
                {/if}
                <details class="mt-1.5">
                  <summary class="cursor-pointer text-xs text-ink-faint hover:text-ink">
                    {t("network.why")}
                  </summary>
                  <p class="mt-1 text-xs leading-relaxed text-ink-muted">{row.reason}</p>
                </details>
              </div>
              <button
                type="button"
                class="btn-ghost shrink-0 px-3! py-1! text-xs"
                onclick={() => copy(row)}
              >
                {copiedKey === keyOf(row) ? t("network.copied") : t("network.copy")}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <section class="flex flex-col items-start gap-3 rounded-xl border border-surface-2 bg-surface-1 p-6">
      <span class="text-xs font-medium uppercase tracking-wider text-ink-faint">
        {t("network.flushTitle")}
      </span>
      <p class="text-sm leading-relaxed text-ink-muted">{t("network.flushHint")}</p>
      <button
        type="button"
        class="btn-outline px-3! py-1.5!"
        disabled={flushing}
        onclick={() => (confirmFlush = true)}
      >
        {flushing ? t("network.flushing") : t("network.flush")}
      </button>
      {#if flushReport !== null}
        <p class="text-sm" role="status">{flushReport}</p>
      {/if}
    </section>
  {/if}
</div>

<ConfirmDialog
  open={confirmFlush}
  title={t("network.flushConfirmTitle")}
  message={t("network.flushConfirmBody")}
  confirmLabel={t("network.flush")}
  onconfirm={flush}
  oncancel={() => (confirmFlush = false)}
/>
