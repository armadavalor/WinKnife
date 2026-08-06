<!--
  Navigation rail. Purely presentational: it renders whatever manifests it is
  handed, so the module registry in Rust is the single source of truth for what
  the app can do.
-->
<script lang="ts">
  import { page } from "$app/state";

  import ModuleIcon from "$lib/components/ModuleIcon.svelte";
  import { t, tModuleName } from "$lib/i18n";
  import type { ModuleManifest } from "$lib/types";

  interface Props {
    /** Modules to list, in registry order. */
    modules: ModuleManifest[];
    /** Whether the registry is still being fetched. */
    loading?: boolean;
  }

  let { modules, loading = false }: Props = $props();

  function isActive(href: string): boolean {
    return page.url.pathname === href;
  }

  const linkClass =
    "flex items-center gap-3 rounded-lg px-3 py-2 text-sm transition-colors";
</script>

<nav
  class="flex w-60 shrink-0 flex-col gap-6 border-r border-surface-2 bg-surface-1 p-4"
  aria-label={t("nav.tools")}
>
  <div class="px-1">
    <p class="text-base font-semibold tracking-tight text-ink">
      {t("app.name")}
    </p>
    <p class="text-xs text-ink-faint">{t("app.tagline")}</p>
  </div>

  <ul class="flex flex-col gap-1">
    <li>
      <a
        href="/dashboard"
        class="{linkClass} {isActive('/dashboard')
          ? 'bg-surface-3 text-ink'
          : 'text-ink-muted hover:bg-surface-2 hover:text-ink'}"
        aria-current={isActive("/dashboard") ? "page" : undefined}
      >
        <ModuleIcon name="dashboard" />
        {t("nav.dashboard")}
      </a>
    </li>
  </ul>

  <div class="flex flex-col gap-2">
    <p
      class="px-3 text-[0.7rem] font-medium uppercase tracking-wider text-ink-faint"
    >
      {t("nav.tools")}
    </p>

    {#if loading}
      <p class="px-3 text-sm text-ink-faint">{t("dashboard.loading")}</p>
    {:else if modules.length === 0}
      <p class="px-3 text-sm text-ink-faint">{t("dashboard.empty")}</p>
    {:else}
      <ul class="flex flex-col gap-1">
        {#each modules as module (module.id)}
          {@const href = `/${module.id}`}
          <li>
            <a
              {href}
              class="{linkClass} {isActive(href)
                ? 'bg-surface-3 text-ink'
                : 'text-ink-muted hover:bg-surface-2 hover:text-ink'}"
              aria-current={isActive(href) ? "page" : undefined}
            >
              <ModuleIcon name={module.icon} />
              {tModuleName(module.id, module.name)}
            </a>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <p class="mt-auto px-3 text-[0.7rem] text-ink-faint">
    {t("status.phase")}
  </p>
</nav>
