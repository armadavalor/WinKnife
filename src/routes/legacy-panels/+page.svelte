<!--
  Legacy panels route (module spec 10).

  A fixed backend catalogue of Windows' own classic windows, grouped. The page
  sends back only an id; WinKnife changes nothing, Windows opens the window
  (and shows its own UAC prompt where one is needed).
-->
<script lang="ts">
  import { onMount } from "svelte";

  import { legacyPanelsList, legacyPanelsOpen } from "$lib/api";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { isApiError, type PanelGroup, type PanelInfo } from "$lib/types";

  const MODULE_ID = "legacy-panels";
  const GROUP_ORDER: readonly PanelGroup[] = [
    "system",
    "hardware",
    "network",
    "apps",
    "diagnostics",
  ];

  const manifest = $derived(moduleStore.find(MODULE_ID));

  let panels = $state<PanelInfo[] | null>(null);
  let opening = $state<string | null>(null);
  let error = $state<string | null>(null);

  const groups = $derived(
    GROUP_ORDER.map((group) => ({
      group,
      entries: (panels ?? []).filter((panel) => panel.group === group),
    })).filter((section) => section.entries.length > 0),
  );

  function message(err: unknown): string {
    return isApiError(err) ? err.message : String(err);
  }

  onMount(() => {
    legacyPanelsList()
      .then((list) => {
        panels = list;
      })
      .catch((err: unknown) => {
        error = message(err);
      });
  });

  async function open(id: string): Promise<void> {
    opening = id;
    error = null;
    try {
      await legacyPanelsOpen(id);
    } catch (err: unknown) {
      error = message(err);
    } finally {
      opening = null;
    }
  }
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(manifest.id, manifest.name) : MODULE_ID}
    info={manifest ? tModuleDescription(manifest.id, manifest.description) : undefined}
  />

  <p class="text-xs leading-relaxed text-ink-muted">{t("legacyPanels.notice")}</p>

  {#if error}
    <p class="rounded-xl border border-risk-danger/30 p-4 text-sm text-risk-danger" role="alert">
      {error}
    </p>
  {/if}

  {#if panels === null && !error}
    <p class="text-sm text-ink-faint">{t("legacyPanels.loading")}</p>
  {/if}

  {#each groups as section (section.group)}
    <section class="flex flex-col gap-3">
      <h2 class="text-xs font-medium uppercase tracking-wider text-ink-faint">
        {t(`legacyPanels.group.${section.group}`)}
      </h2>
      <ul class="grid grid-cols-1 gap-3 md:grid-cols-2">
        {#each section.entries as panel (panel.id)}
          <li
            class="flex items-start justify-between gap-4 rounded-xl border border-surface-2 bg-surface-1 p-4"
          >
            <div class="flex min-w-0 flex-col gap-1">
              <span class="text-sm font-medium">{panel.name}</span>
              <span class="text-xs leading-relaxed text-ink-muted">{panel.description}</span>
              {#if !panel.available}
                <span class="text-xs text-ink-faint">{t("legacyPanels.unavailable")}</span>
              {/if}
            </div>
            <button
              type="button"
              class="shrink-0 rounded-lg border border-surface-3 px-3 py-1.5 text-sm transition-colors hover:bg-surface-2 disabled:cursor-not-allowed disabled:opacity-40"
              disabled={!panel.available || opening !== null}
              onclick={() => open(panel.id)}
            >
              {t("legacyPanels.open")}
            </button>
          </li>
        {/each}
      </ul>
    </section>
  {/each}
</div>
