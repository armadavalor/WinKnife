<!--
  Dashboard: placeholder cards, one per registered module.

  Phase 0 has no features, so the cards only prove that the sidebar and this
  grid are both driven by `list_modules`.
-->
<script lang="ts">
  import ModuleIcon from "$lib/components/ModuleIcon.svelte";
  import {
    t,
    tModuleDescription,
    tModuleName,
  } from "$lib/i18n";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import type { ModuleManifest } from "$lib/types";

  function capabilityLabels(module: ModuleManifest): string[] {
    const labels: string[] = [];
    if (module.capabilities.hasScan) {
      labels.push(t("module.capability.scan"));
    }
    if (module.capabilities.hasExecute) {
      labels.push(t("module.capability.execute"));
    }
    if (module.capabilities.hasSettings) {
      labels.push(t("module.capability.settings"));
    }
    return labels;
  }
</script>

<div class="mx-auto flex max-w-4xl flex-col gap-8 px-8 py-10">
  <header class="flex flex-col gap-2">
    <h1 class="text-2xl font-semibold tracking-tight">
      {t("dashboard.title")}
    </h1>
    <p class="max-w-2xl text-sm leading-relaxed text-ink-muted">
      {t("dashboard.subtitle")}
    </p>
  </header>

  {#if moduleStore.status === "loading" || moduleStore.status === "idle"}
    <p class="text-sm text-ink-faint">{t("dashboard.loading")}</p>
  {:else if moduleStore.status === "error"}
    <div
      class="flex flex-col items-start gap-3 rounded-xl border border-surface-3 bg-surface-1 p-5"
    >
      <p class="text-sm text-ink">{t("dashboard.error")}</p>
      {#if moduleStore.error}
        <p class="font-mono text-xs text-ink-faint">{moduleStore.error}</p>
      {/if}
      <button
        type="button"
        class="rounded-lg bg-surface-3 px-3 py-1.5 text-sm text-ink transition-colors hover:bg-accent-soft"
        onclick={() => moduleStore.reload()}
      >
        {t("dashboard.retry")}
      </button>
    </div>
  {:else if moduleStore.modules.length === 0}
    <p class="text-sm text-ink-faint">{t("dashboard.empty")}</p>
  {:else}
    <ul class="grid grid-cols-1 gap-4 sm:grid-cols-2">
      {#each moduleStore.modules as module (module.id)}
        <li>
          <a
            href="/{module.id}"
            class="flex h-full flex-col gap-3 rounded-xl border border-surface-2 bg-surface-1 p-5 transition-colors hover:border-accent-soft hover:bg-surface-2"
          >
            <span class="flex items-center gap-3">
              <ModuleIcon name={module.icon} class="size-5 text-accent" />
              <span class="font-medium">
                {tModuleName(module.id, module.name)}
              </span>
            </span>

            <span class="text-sm leading-relaxed text-ink-muted">
              {tModuleDescription(module.id, module.description)}
            </span>

            {#if capabilityLabels(module).length > 0}
              <span class="mt-auto flex flex-wrap gap-2 pt-1">
                {#each capabilityLabels(module) as label (label)}
                  <span
                    class="rounded-md bg-surface-3 px-2 py-0.5 text-[0.7rem] text-ink-muted"
                  >
                    {label}
                  </span>
                {/each}
              </span>
            {/if}
          </a>
        </li>
      {/each}
    </ul>
  {/if}
</div>
