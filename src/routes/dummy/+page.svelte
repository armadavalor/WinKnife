<!--
  Route for the dummy module. One folder per module, named after the module id
  (ARCHITECTURE section 1). Phase 0 renders the manifest and nothing else.
-->
<script lang="ts">
  import ModuleIcon from "$lib/components/ModuleIcon.svelte";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { moduleStore } from "$lib/stores/modules.svelte";

  const MODULE_ID = "dummy";

  const manifest = $derived(moduleStore.find(MODULE_ID));
</script>

<div class="mx-auto flex max-w-4xl flex-col gap-6 px-8 py-10">
  {#if manifest}
    <header class="flex items-center gap-3">
      <ModuleIcon name={manifest.icon} class="size-6 text-accent" />
      <h1 class="text-2xl font-semibold tracking-tight">
        {tModuleName(manifest.id, manifest.name)}
      </h1>
    </header>

    <p class="max-w-2xl text-sm leading-relaxed text-ink-muted">
      {tModuleDescription(manifest.id, manifest.description)}
    </p>
  {:else}
    <h1 class="text-2xl font-semibold tracking-tight">{MODULE_ID}</h1>
  {/if}

  <p
    class="rounded-xl border border-dashed border-surface-3 p-5 text-sm text-ink-faint"
  >
    {t("module.placeholder")}
  </p>
</div>
