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
  import { slide } from "svelte/transition";
  import { categoryOf, groupByCategory } from "$lib/categories";
  import { t, tModuleName } from "$lib/i18n";
  import { elevation } from "$lib/stores/elevation.svelte";
  import { motion } from "$lib/stores/motion.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import type { ModuleCategory, ModuleManifest } from "$lib/types";

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

  const OPEN_KEY = "winknife.sidebarOpenGroups";

  function readOpen(): string[] {
    try {
      const raw = JSON.parse(localStorage.getItem(OPEN_KEY) ?? "[]");
      return Array.isArray(raw) ? raw.filter((v) => typeof v === "string") : [];
    } catch {
      return [];
    }
  }

  let openGroups = $state<string[]>(readOpen());

  const groups = $derived(groupByCategory(modules));

  function setOpen(category: string, open: boolean) {
    const rest = openGroups.filter((c) => c !== category);
    openGroups = open ? [...rest, category] : rest;
    try {
      localStorage.setItem(OPEN_KEY, JSON.stringify(openGroups));
    } catch {
      // Remembering open groups is a convenience; the sidebar works without it.
    }
  }

  // Opening a tool (link, dashboard card, `--module`) expands its group once
  // per route, so collapsing it afterwards sticks.
  let syncedPath: string | null = null;
  $effect(() => {
    const path = page.url.pathname;
    if (path === syncedPath) return;
    const active = modules.find((m) => path === `/${m.id}`);
    if (!active) return;
    syncedPath = path;
    const category = categoryOf(active);
    if (!openGroups.includes(category)) setOpen(category, true);
  });

  /** Instant when animations are off; `slide` otherwise. */
  const reveal = (node: Element) => slide(node, { duration: motion.enabled ? 150 : 0 });

  const linkClass =
    "flex items-center gap-3 rounded-lg px-3 py-2 text-sm transition-colors";
</script>

<nav
  class="flex w-60 shrink-0 flex-col gap-6 overflow-y-auto border-r border-surface-2 bg-surface-1/70 p-4 backdrop-blur-xl"
  aria-label={t("nav.tools")}
>
  <a
    href="/dashboard"
    class="group flex items-center gap-3 rounded-lg px-1 py-1 outline-none focus-visible:outline-2 focus-visible:outline-accent"
    aria-label={t("app.name")}
  >
    <KnifeLogo size={48} {loading} />
    <div class="min-w-0 leading-tight">
      <p class="truncate text-lg font-semibold tracking-tight text-ink">
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

  {#if elevation.elevated !== null}
    <div class="flex flex-wrap items-center gap-2 px-1">
      <span
        class="rounded-full px-2 py-0.5 text-[0.7rem] font-medium {elevation.elevated
          ? 'bg-accent-soft text-ink'
          : 'bg-surface-3 text-ink-muted'}"
        title={elevation.elevated ? t("elevation.adminHint") : t("elevation.standardHint")}
        data-testid="elevation-badge"
      >
        {elevation.elevated ? t("elevation.admin") : t("elevation.standard")}
      </span>
      {#if !elevation.elevated}
        <button
          type="button"
          class="rounded-md px-1.5 py-0.5 text-[0.7rem] text-ink-muted outline-none hover:bg-surface-2 hover:text-ink hover:shadow-sm focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-60"
          disabled={elevation.busy}
          onclick={() => void elevation.restart(page.url.pathname)}
        >
          {elevation.busy ? t("elevation.restarting") : t("elevation.restart")}
        </button>
      {/if}
    </div>
  {/if}

  <ul class="flex flex-col gap-1">
    <li>
      <a
        href="/dashboard"
        class="{linkClass} {isActive('/dashboard')
          ? 'bg-surface-3 text-ink shadow-sm'
          : 'text-ink-muted hover:bg-surface-2 hover:text-ink hover:shadow-sm'}"
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
      <ul class="flex flex-col gap-1" aria-label={t("nav.categories")}>
        {#each groups as group (group.category)}
          {@const isOpen = openGroups.includes(group.category)}
          <li>
            <button
              type="button"
              id="sidebar-group-{group.category}"
              aria-expanded={isOpen}
              aria-controls="sidebar-tools-{group.category}"
              data-tools={group.modules.map((m) => m.id).join(" ")}
              class="flex w-full items-center gap-2 rounded-lg px-3 py-1.5 text-sm font-medium text-ink-muted outline-none transition-colors hover:bg-surface-2 hover:text-ink hover:shadow-sm focus-visible:outline-2 focus-visible:outline-accent"
              onclick={() => setOpen(group.category, !isOpen)}
            >
              <Icon
                name="chevron-right"
                class="size-4 shrink-0 transition-transform {isOpen ? 'rotate-90' : ''}"
              />
              <span class="flex-1 text-left">{t(`nav.category.${group.category}`)}</span>
            </button>
            {#if isOpen}
              <ul
                class="mt-1 flex flex-col gap-1"
                id="sidebar-tools-{group.category}"
                transition:reveal
              >
                {#each group.modules as module (module.id)}
                  {@const href = `/${module.id}`}
                  <li>
                    <a
                      {href}
                      class="{linkClass} {isActive(href)
                        ? 'bg-surface-3 text-ink shadow-sm'
                        : 'text-ink-muted hover:bg-surface-2 hover:text-ink hover:shadow-sm'}"
                      aria-current={isActive(href) ? "page" : undefined}
                    >
                      <ModuleIcon name={module.icon} />
                      {tModuleName(module.id, module.name)}
                    </a>
                  </li>
                {/each}
              </ul>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <div class="mt-auto flex flex-col gap-1">
    <a
      href="/settings"
      class="{linkClass} {isActive('/settings')
        ? 'bg-surface-3 text-ink shadow-sm'
        : 'text-ink-muted hover:bg-surface-2 hover:text-ink hover:shadow-sm'}"
      aria-current={isActive("/settings") ? "page" : undefined}
    >
      <ModuleIcon name="settings" />
      {t("nav.settings")}
    </a>

    <button
      type="button"
      class="{linkClass} w-full text-ink-muted hover:bg-surface-2 hover:text-ink hover:shadow-sm"
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
