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

  import { goto } from "$app/navigation";

  import { onLaunchNavigate, takeLaunchRoute } from "$lib/api";
  import QuarantineDrawer from "$lib/components/QuarantineDrawer.svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import { t } from "$lib/i18n";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { motion } from "$lib/stores/motion.svelte";
  import { quarantineStore } from "$lib/stores/quarantine.svelte";
  import { theme } from "$lib/stores/theme.svelte";

  let { children } = $props();

  // Mounting is a one-shot, so it is not an effect: an effect subscribes to
  // whatever its body reads, and a store that reports its own progress would
  // then re-trigger the very call that moved it.
  onMount(() => {
    theme.init();
    motion.init();
    void moduleStore.load();
    void quarantineStore.reload();
    void openLaunchRoute();
  });

  /**
   * Open the module a launch asked for (`--module <id>`): the first launch's
   * on mount, a forwarded one's on `launch:navigate`. The route is taken, so
   * it opens once whichever of the two reads it first.
   */
  async function openLaunchRoute(): Promise<void> {
    try {
      const route = await takeLaunchRoute();
      if (route) await goto(route);
    } catch (err) {
      // Best-effort: the app is usable without it; leave the current page.
      console.warn("the launch route could not be opened", err);
    }
  }

  $effect(() => {
    const listening = untrack(() => onLaunchNavigate(() => void openLaunchRoute()));
    return () => {
      void listening.then((unlisten) => unlisten());
    };
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

<div class="relative isolate flex h-screen w-screen overflow-hidden">
  <!-- Decorative: accent glow, then film grain. Neither takes clicks or layout. -->
  <div class="bg-glow pointer-events-none fixed inset-0 -z-10" aria-hidden="true"></div>
  <svg class="bg-grain pointer-events-none fixed inset-0 -z-10 h-full w-full" aria-hidden="true" focusable="false">
    <defs>
      <filter id="wk-grain" x="0" y="0" width="100%" height="100%" color-interpolation-filters="sRGB">
        <feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="2" seed="7" stitchTiles="stitch" />
        <feColorMatrix type="saturate" values="0" />
      </filter>
      <pattern id="wk-grain-tile" width="256" height="256" patternUnits="userSpaceOnUse">
        <rect width="256" height="256" filter="url(#wk-grain)" />
      </pattern>
    </defs>
    <rect width="100%" height="100%" fill="url(#wk-grain-tile)" />
  </svg>

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
