<!--
  Installed apps: a read-only inventory of Win32 programs and Store packages.

  There is deliberately no uninstall button. The two actions are "open Windows
  Settings" and "show the install folder". Signer lookups are lazy: a per-row
  button, plus one "check shown" capped at CHECK_CAP rows so a long list never
  fires hundreds of signature checks at once.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import EmptyState from "$lib/components/EmptyState.svelte";
  import ListToolbar from "$lib/components/ListToolbar.svelte";
  import LoadingNote from "$lib/components/LoadingNote.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { appsList, appsOpenSettings, appsSigner, revealInExplorer } from "$lib/api";
  import { formatBytes } from "$lib/format";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { matchesFilter, sortRows, type SortKey } from "$lib/listTools";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { isApiError, type AppEntry, type AppSource, type SignerInfo } from "$lib/types";

  const MODULE_ID = "apps";
  const CHECK_CAP = 40;
  const SOURCES: AppSource[] = ["win32", "store"];
  const manifest = $derived(moduleStore.find(MODULE_ID));

  let entries = $state<AppEntry[]>([]);
  let notes = $state<string[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let filter = $state("");
  let sort = $state<SortKey>("name");
  let signers = $state<Record<string, SignerInfo | "busy">>({});

  const shown = $derived(
    sortRows(
      entries.filter((e) => matchesFilter(filter, e.name, e.publisher, e.family)),
      sort,
      (e) => e.sizeKb ?? 0,
      (e) => e.name,
    ),
  );
  const checkable = $derived(
    shown.filter((e) => e.canCheckSigner && signers[e.id] === undefined).slice(0, CHECK_CAP),
  );

  function message(e: unknown): string {
    return isApiError(e) ? e.message : String(e);
  }

  async function load(): Promise<void> {
    try {
      const list = await appsList();
      entries = list.entries;
      notes = list.notes;
    } catch (e) {
      error = message(e);
    } finally {
      loading = false;
    }
  }

  async function check(entry: AppEntry): Promise<void> {
    signers[entry.id] = "busy";
    try {
      signers[entry.id] = await appsSigner(entry.id);
    } catch {
      signers[entry.id] = { state: "unknown", publisher: null };
    }
  }

  async function checkShown(): Promise<void> {
    for (const entry of checkable) await check(entry);
  }

  async function run(action: () => Promise<void>): Promise<void> {
    error = null;
    try {
      await action();
    } catch (e) {
      error = message(e);
    }
  }

  onMount(() => {
    void load();
  });
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(manifest.id, manifest.name) : MODULE_ID}
    info={manifest ? tModuleDescription(manifest.id, manifest.description) : ""}
  />
  <p class="rounded-lg bg-surface-2 p-3 text-xs leading-relaxed text-ink-muted">
    {t("apps.notice")}
  </p>

  {#if error}
    <p role="alert" class="rounded-xl border border-risk-danger/30 bg-surface-1 p-4 text-sm text-risk-danger">
      {error}
    </p>
  {/if}

  {#if loading}
    <LoadingNote text={t("apps.loading")} />
  {:else if entries.length === 0}
    <EmptyState message={t("apps.empty")} />
  {:else}
    <div class="flex flex-wrap items-center gap-3 text-sm text-ink-muted">
      <span>{t("apps.summary", { total: entries.length })}</span>
      <button type="button" class="btn-outline px-2.5! py-1! text-xs!" onclick={() => run(appsOpenSettings)}>
        {t("apps.openSettings")}
      </button>
      <button
        type="button"
        class="btn-outline px-2.5! py-1! text-xs!"
        disabled={checkable.length === 0}
        onclick={() => checkShown()}
      >
        {t("apps.checkShown", { count: checkable.length })}
      </button>
    </div>

    {#if notes.length > 0}
      <p class="rounded-lg bg-surface-2 p-3 text-xs leading-relaxed text-ink-muted">
        {t("apps.unreadable")}
        <span class="font-mono">{notes.join(" · ")}</span>
      </p>
    {/if}

    <ListToolbar bind:filter bind:sort shown={shown.length} total={entries.length} />

    {#if shown.length === 0}
      <EmptyState message={t("list.noMatch")} />
    {/if}

    {#each SOURCES as source (source)}
      {@const rows = shown.filter((e) => e.source === source)}
      {#if rows.length > 0}
        <section class="flex flex-col gap-2">
          <h2 class="text-sm font-semibold text-ink-muted">
            {t(`apps.group.${source}`)} ({rows.length})
          </h2>
          <ul class="flex flex-col gap-2">
            {#each rows as entry (entry.id)}
              {@const signer = signers[entry.id]}
              <li class="flex flex-wrap items-start gap-4 rounded-xl border border-surface-2 bg-surface-1 p-4">
                <div class="min-w-0 flex-1">
                  <p class="flex flex-wrap items-center gap-2">
                    <span class="font-medium">{entry.name}</span>
                    {#if entry.kind !== "app"}
                      <span class="rounded-md bg-surface-2 px-1.5 py-0.5 text-[0.65rem] text-ink-faint">
                        {t(`apps.kind.${entry.kind}`)}
                      </span>
                    {/if}
                  </p>
                  <p class="mt-1 text-xs text-ink-muted">
                    {[
                      entry.publisher,
                      entry.version,
                      entry.sizeKb ? formatBytes(entry.sizeKb * 1024) : null,
                      entry.installDate ? t("apps.installed", { date: entry.installDate }) : null,
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </p>
                  {#if entry.installFolder}
                    <p class="mt-1 truncate font-mono text-[0.7rem] text-ink-faint" title={entry.installFolder}>
                      {entry.installFolder}
                    </p>
                  {/if}
                  {#if signer && signer !== "busy"}
                    <p
                      class="mt-1.5 text-xs {signer.state === 'unsigned'
                        ? 'text-risk-review'
                        : 'text-ink-muted'}"
                    >
                      {#if signer.state === "signed"}
                        {signer.publisher
                          ? t("apps.signed", { name: signer.publisher })
                          : t("apps.signedUnnamed")}
                      {:else if signer.state === "unsigned"}
                        {t("apps.unsigned")}
                      {:else}
                        {t("apps.signerUnknown")}
                      {/if}
                    </p>
                  {/if}
                </div>
                <div class="flex shrink-0 flex-col items-end gap-1.5">
                  {#if entry.installFolder}
                    {@const folder = entry.installFolder}
                    <button
                      type="button"
                      class="btn-outline px-2.5! py-1! text-xs!"
                      onclick={() => run(() => revealInExplorer(folder))}
                    >
                      {t("apps.showFolder")}
                    </button>
                  {/if}
                  {#if entry.canCheckSigner && signer === undefined}
                    <button type="button" class="btn-outline px-2.5! py-1! text-xs!" onclick={() => check(entry)}>
                      {t("apps.checkSigner")}
                    </button>
                  {:else if signer === "busy"}
                    <span class="text-xs text-ink-faint">{t("apps.checking")}</span>
                  {/if}
                </div>
              </li>
            {/each}
          </ul>
        </section>
      {/if}
    {/each}
  {/if}
</div>
