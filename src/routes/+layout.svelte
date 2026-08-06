<!--
  App shell: navigation rail on the left, routed content on the right.

  The registry is fetched once here so the sidebar and every route share one
  copy of it.
-->
<script lang="ts">
  import "../app.css";

  import Sidebar from "$lib/components/Sidebar.svelte";
  import { t } from "$lib/i18n";
  import { moduleStore } from "$lib/stores/modules.svelte";

  let { children } = $props();

  $effect(() => {
    void moduleStore.load();
  });
</script>

<a
  href="#main"
  class="sr-only focus:not-sr-only focus:absolute focus:left-4 focus:top-4 focus:z-10 focus:rounded-lg focus:bg-surface-3 focus:px-3 focus:py-2 focus:text-sm"
>
  {t("nav.skipToContent")}
</a>

<div class="flex h-screen w-screen overflow-hidden">
  <Sidebar
    modules={moduleStore.modules}
    loading={moduleStore.status === "loading"}
  />

  <main id="main" class="flex-1 overflow-y-auto">
    {@render children()}
  </main>
</div>
