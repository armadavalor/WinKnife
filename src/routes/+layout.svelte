<!--
  App shell: navigation rail on the left, routed content on the right, and the
  quarantine drawer overlaid on top of both.

  The registry is fetched once here so the sidebar and every route share one
  copy of it. The drawer subscribes to `quarantine:changed` from the shell, so
  its badge stays honest whichever route is open when a clean finishes.
-->
<script lang="ts">
  import "../app.css";

  import { onMount, untrack } from "svelte";

  import QuarantineDrawer from "$lib/components/QuarantineDrawer.svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import { t } from "$lib/i18n";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { quarantineStore } from "$lib/stores/quarantine.svelte";

  let { children } = $props();

  // Mounting is a one-shot, so it is not an effect: an effect subscribes to
  // whatever its body reads, and a store that reports its own progress would
  // then re-trigger the very call that moved it.
  onMount(() => {
    void moduleStore.load();
    void quarantineStore.reload();
  });

  $effect(() => {
    // `untrack` so nothing the subscription touches on the way in can become a
    // dependency of this effect; the teardown is the only reason it is one.
    const listening = untrack(() => quarantineStore.listen());
    return () => {
      void listening.then((unlisten) => unlisten());
    };
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
    quarantineCount={quarantineStore.stats.itemCount}
    onopenquarantine={() => quarantineStore.show()}
  />

  <main id="main" class="flex-1 overflow-y-auto">
    {@render children()}
  </main>
</div>

<QuarantineDrawer />
