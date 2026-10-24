<!--
  Navigation rail. Purely presentational: it renders whatever manifests it is
  handed, so the module registry in Rust is the single source of truth for what
  the app can do.
-->
<script lang="ts">
  import LoadingNote from "$lib/components/LoadingNote.svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { onMount } from "svelte";

  import { page } from "$app/state";

  import KnifeLogo from "$lib/components/KnifeLogo.svelte";
  import ModuleIcon from "$lib/components/ModuleIcon.svelte";
  import { t, tModuleName } from "$lib/i18n";
  import type { ModuleManifest } from "$lib/types";

  interface Props {
    /** Modules to list, in registry order. */
    modules: ModuleManifest[];
    /** Whether the registry is still being fetched. */
    loading?: boolean;
    /** Items currently held in quarantine, shown as a badge. */
    quarantineCount?: number;
    /** Called when the quarantine entry is clicked. */
    onopenquarantine?: () => void;
    /** Shown instead of the usual empty note when `modules` is empty. */
    emptyText?: string;
  }

  let {
    modules,
    loading = false,
    quarantineCount = 0,
    onopenquarantine,
    emptyText,
  }: Props = $props();

  let version = $state<string | null>(null);

  onMount(async () => {
    try {
      version = await getVersion();
    } catch {
      version = null;
    }
  });

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
  <a
    href="/dashboard"
    class="group flex items-center gap-3 rounded-lg px-1 py-1 outline-none focus-visible:outline-2 focus-visible:outline-accent"
    aria-label={t("app.name")}
  >
    <KnifeLogo size={32} {loading} />
    <div class="min-w-0 leading-tight">
      <p class="truncate text-base font-semibold tracking-tight text-ink">
        {t("app.name")}
      </p>
      {#if version}
        <p class="text-[0.7rem] tabular-nums text-ink-faint">
          {t("status.version", { version })}
        </p>
      {:else}
        <p class="text-[0.7rem] text-ink-faint">{t("app.tagline")}</p>
      {/if}
    </div>
  </a>

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
      <LoadingNote text={t("dashboard.loading")} class="px-3" />
    {:else if modules.length === 0}
      <p class="px-3 text-sm text-ink-faint">{emptyText ?? t("dashboard.empty")}</p>
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

  <div class="mt-auto flex flex-col gap-1">
    <a
      href="/settings"
      class="{linkClass} {isActive('/settings')
        ? 'bg-surface-3 text-ink'
        : 'text-ink-muted hover:bg-surface-2 hover:text-ink'}"
      aria-current={isActive("/settings") ? "page" : undefined}
    >
      <ModuleIcon name="settings" />
      {t("nav.settings")}
    </a>

    <button
      type="button"
      class="{linkClass} w-full text-ink-muted hover:bg-surface-2 hover:text-ink"
      onclick={() => onopenquarantine?.()}
    >
      <ModuleIcon name="shield" />
      <span class="flex-1 text-left">{t("quarantine.open")}</span>
      {#if quarantineCount > 0}
        <span
          class="rounded-md bg-surface-3 px-1.5 py-0.5 text-[0.7rem] tabular-nums text-ink-muted"
        >
          {quarantineCount}
        </span>
      {/if}
    </button>
  </div>
</nav>
