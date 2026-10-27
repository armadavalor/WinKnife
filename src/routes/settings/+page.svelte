<!--
  Settings: per-module storage, rendered from what Rust says it has.

  The pack list is not hard-coded — it is whatever loaded on this machine, so a
  community pack dropped into `rules/` appears here with no UI change. And the
  numeric fields show what was *stored*, which is not always what was typed:
  Rust clamps retention and the duplicate size floor, and the page says so.
-->
<script lang="ts">
  import { panelOpacity } from "$lib/stores/panelOpacity.svelte";
  import { OPACITY_MAX, OPACITY_MIN } from "$lib/panelOpacity";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import LoadingNote from "$lib/components/LoadingNote.svelte";
  import { onMount } from "svelte";

  import PageHeader from "$lib/components/PageHeader.svelte";
  import {
    execute,
    getSettings,
    pickProfileOpen,
    pickProfileSave,
    profileExport,
    profilePreview,
    setSettings,
  } from "$lib/api";
  import { formatBytes } from "$lib/format";
  import { t, tModuleName, type TranslationKey } from "$lib/i18n";
  import { hiddenModules } from "$lib/stores/hiddenModules.svelte";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { settingsStore } from "$lib/stores/settings.svelte";
  import { ANIMATION_PREFS } from "$lib/motion";
  import { BACKDROPS } from "$lib/sky";
  import { backdrop } from "$lib/stores/backdrop.svelte";
  import { GLOW_VARIANTS } from "$lib/glow";
  import { glow } from "$lib/stores/glow.svelte";
  import { locale } from "$lib/stores/locale.svelte";
  import type { Locale } from "$lib/localePref";

  // Each language is named in itself, so it stays readable from the wrong one.
  const LANGUAGE_CHOICES: { id: Locale; label: string }[] = [
    { id: "en", label: "English" },
    { id: "tr", label: "Türkçe" },
  ];
  import { motion } from "$lib/stores/motion.svelte";
  import { THEMES, theme } from "$lib/stores/theme.svelte";
  import { isApiError, type AppSettings, type ProfilePreview } from "$lib/types";

  const store = settingsStore;

  /** Mirrors `MIN_RETENTION_DAYS` / `MAX_RETENTION_DAYS` in core/quarantine.rs. */
  const RETENTION_MIN = 1;
  const RETENTION_MAX = 365;
  /** Mirrors `MIN_ALLOWED_SIZE_BYTES` in modules/duplicates.rs. */
  const MIN_DUPLICATE_BYTES = 4 * 1024;

  const MIN_SIZE_CHOICES = [
    { value: 4 * 1024, label: "4 KB" },
    { value: 256 * 1024, label: "256 KB" },
    { value: 1024 * 1024, label: "1 MB" },
    { value: 10 * 1024 * 1024, label: "10 MB" },
    { value: 100 * 1024 * 1024, label: "100 MB" },
  ];

  const CAP_CHOICES = [
    { value: 1024 * 1024 * 1024, label: "1 GB" },
    { value: 4 * 1024 * 1024 * 1024, label: "4 GB" },
    { value: 16 * 1024 * 1024 * 1024, label: "16 GB" },
    { value: 64 * 1024 * 1024 * 1024, label: "64 GB" },
  ];

  /**
   * Preview colours per theme (page, card, accent). Hardcoded on purpose: the
   * live CSS variables only reflect the active theme, and each card must show
   * its own.
   */
  const SWATCHES = {
    graphite: ["oklch(16% 0.006 265)", "oklch(25% 0.008 265)", "oklch(72% 0.13 205)"],
    forest: ["oklch(15% 0.014 158)", "oklch(24% 0.018 158)", "oklch(76% 0.11 182)"],
    mist: ["oklch(96% 0.004 265)", "oklch(87.5% 0.007 265)", "oklch(48% 0.12 205)"],
  } as const;

  /** A checked profile and the settings it would replace; nothing applied yet. */
  let preview = $state<{ profile: ProfilePreview; changes: string[] } | null>(null);
  let confirming = $state(false);
  let profileBusy = $state(false);
  let profileNote = $state<string | null>(null);
  let profileError = $state<string | null>(null);
  let profileFailures = $state<string[]>([]);

  function errorText(error: unknown): string {
    return isApiError(error) ? error.message : String(error);
  }

  function resetProfileMessages(): void {
    profileNote = null;
    profileError = null;
    profileFailures = [];
  }

  /** What saving `to` over the stored `from` would change, in words. */
  function settingsChanges(from: AppSettings, to: AppSettings): string[] {
    const changes: string[] = [];
    for (const pack of to.cleaner.packs) {
      const before = from.cleaner.packs.find((p) => p.id === pack.id)?.enabled;
      if (before !== pack.enabled) {
        changes.push(
          t(pack.enabled ? "settings.profileDiffPackOn" : "settings.profileDiffPackOff", {
            name: pack.name,
          }),
        );
      }
    }
    if (from.duplicates.minSizeBytes !== to.duplicates.minSizeBytes) {
      changes.push(
        t("settings.profileDiffMinSize", {
          from: formatBytes(from.duplicates.minSizeBytes),
          to: formatBytes(to.duplicates.minSizeBytes),
        }),
      );
    }
    if (JSON.stringify(from.duplicates.roots) !== JSON.stringify(to.duplicates.roots)) {
      changes.push(t("settings.profileDiffRoots", { count: to.duplicates.roots.length }));
    }
    if (from.quarantine.retentionDays !== to.quarantine.retentionDays) {
      changes.push(
        t("settings.profileDiffRetention", {
          from: from.quarantine.retentionDays,
          to: to.quarantine.retentionDays,
        }),
      );
    }
    if (from.quarantine.maxBytes !== to.quarantine.maxBytes) {
      changes.push(
        t("settings.profileDiffMaxBytes", {
          from: formatBytes(from.quarantine.maxBytes),
          to: formatBytes(to.quarantine.maxBytes),
        }),
      );
    }
    return changes;
  }

  async function exportProfile(): Promise<void> {
    resetProfileMessages();
    const path = await pickProfileSave();
    if (path === null) return;
    try {
      await profileExport(path);
      profileNote = t("settings.profileExported", { path });
    } catch (error: unknown) {
      profileError = errorText(error);
    }
  }

  async function importProfile(): Promise<void> {
    resetProfileMessages();
    preview = null;
    const path = await pickProfileOpen();
    if (path === null) return;
    profileBusy = true;
    try {
      const [profile, stored] = await Promise.all([profilePreview(path), getSettings()]);
      preview = { profile, changes: settingsChanges(stored, profile.settings) };
    } catch (error: unknown) {
      profileError = errorText(error);
    } finally {
      profileBusy = false;
    }
  }

  // Tweaks go through the ordinary execute path, so the fence and the undo
  // store apply exactly as on the Tweaks page.
  async function applyProfile(): Promise<void> {
    confirming = false;
    const checked = preview?.profile;
    if (!checked) return;
    profileBusy = true;
    try {
      const ids = checked.recommendations.map((rec) => rec.id);
      const report = ids.length > 0 ? await execute(checked.scanId, ids, false) : null;
      profileFailures = (report?.items ?? []).flatMap((item) =>
        item.outcome.status === "failed"
          ? [`${item.path}: ${item.outcome.error}`]
          : item.outcome.status === "skipped"
            ? [`${item.path}: ${item.outcome.reason}`]
            : [],
      );
      await setSettings(checked.settings);
      await store.load();
      profileNote = t("settings.profileApplied");
      preview = null;
    } catch (error: unknown) {
      profileError = errorText(error);
    } finally {
      profileBusy = false;
    }
  }

  // Run-once mounting, deliberately not reactive: as an effect this subscribed
  // to the store's own `status` and reloaded on every transition it caused.
  onMount(() => {
    void store.load();
  });
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader title={t("settings.title")} info={t("settings.subtitle")} />

  <section class="rounded-xl border border-surface-2 bg-surface-1 p-6">
    <h2 class="text-sm font-medium">{t("settings.appearanceTitle")}</h2>
    <p class="mt-1 text-xs text-ink-muted">{t("settings.appearanceSubtitle")}</p>

    <div
      class="mt-4 grid gap-3 sm:grid-cols-3"
      role="radiogroup"
      aria-label={t("settings.themeLabel")}
    >
      {#each THEMES as id (id)}
        {@const [page, card, accent] = SWATCHES[id]}
        <label
          class="flex cursor-pointer items-center gap-3 rounded-xl border border-surface-2 p-3 transition-colors hover:bg-surface-2 has-[:checked]:border-accent has-[:checked]:bg-accent-soft/40 has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-accent"
        >
          <input
            type="radio"
            name="theme"
            value={id}
            class="sr-only"
            checked={theme.current === id}
            onchange={() => theme.set(id)}
          />
          <span
            class="size-10 shrink-0 rounded-lg border border-ink/15"
            style="background: linear-gradient(135deg, {page} 0 50%, {card} 50% 75%, {accent} 75%)"
            aria-hidden="true"
          ></span>
          <span class="min-w-0">
            <span class="block text-sm text-ink">
              {t(`settings.theme.${id}` as TranslationKey)}
            </span>
            <span class="block text-xs text-ink-faint">
              {t(`settings.themeHint.${id}` as TranslationKey)}
            </span>
          </span>
        </label>
      {/each}
    </div>
    <p class="mt-3 text-xs text-ink-faint">{t("settings.themeNote")}</p>

    <div class="mt-5 flex items-center justify-between gap-4 border-t border-surface-2 pt-4">
      <div class="min-w-0">
        <p class="text-sm text-ink">{t("settings.animationsLabel")}</p>
        <p class="text-xs text-ink-faint">
          {motion.reducedMotion && motion.pref === "system"
            ? t("settings.animationsReduced")
            : t("settings.animationsHint")}
        </p>
      </div>
      <div class="flex shrink-0 gap-1" role="radiogroup" aria-label={t("settings.animationsLabel")}>
        {#each ANIMATION_PREFS as id (id)}
          <button
            type="button"
            role="radio"
            aria-checked={motion.pref === id}
            class="rounded-lg px-3 py-1 text-xs font-medium transition-colors {motion.pref === id
              ? 'bg-accent-soft text-ink'
              : 'border border-surface-3 text-ink-muted hover:bg-surface-2'}"
            onclick={() => motion.set(id)}
          >
            {t(`settings.animations.${id}` as TranslationKey)}
          </button>
        {/each}
      </div>
    </div>

    <div class="mt-4 flex flex-wrap items-center justify-between gap-4 border-t border-surface-2 pt-4">
      <div class="min-w-0">
        <p class="text-sm text-ink">{t("settings.backdropLabel")}</p>
        <p class="text-xs text-ink-faint">{t("settings.backdropHint")}</p>
      </div>
      <div class="flex shrink-0 flex-wrap gap-1" role="radiogroup" aria-label={t("settings.backdropLabel")}>
        {#each BACKDROPS as id (id)}
          <button
            type="button"
            role="radio"
            aria-checked={backdrop.value === id}
            class="rounded-lg px-3 py-1 text-xs font-medium transition-colors {backdrop.value === id
              ? 'bg-accent-soft text-ink'
              : 'border border-surface-3 text-ink-muted hover:bg-surface-2'}"
            onclick={() => backdrop.set(id)}
          >
            {t(`settings.backdrop.${id}` as TranslationKey)}
          </button>
        {/each}
      </div>
    </div>

    <div class="mt-4 flex flex-wrap items-center justify-between gap-4 border-t border-surface-2 pt-4">
      <div class="min-w-0">
        <p class="text-sm text-ink">{t("settings.sidebarOpacityLabel")}</p>
        <p class="text-xs text-ink-faint">{t("settings.sidebarOpacityHint")}</p>
      </div>
      <label class="flex shrink-0 items-center gap-3 text-xs text-ink-muted">
        <input
          type="range"
          min={OPACITY_MIN}
          max={OPACITY_MAX}
          step="5"
          class="w-40 accent-[var(--color-accent)]"
          aria-label={t("settings.sidebarOpacityLabel")}
          value={panelOpacity.sidebar}
          oninput={(e) => panelOpacity.setSidebar(e.currentTarget.valueAsNumber)}
        />
        <span class="w-10 text-right tabular-nums">{panelOpacity.sidebar}%</span>
      </label>
    </div>

    <div class="mt-4 flex flex-wrap items-center justify-between gap-4 border-t border-surface-2 pt-4">
      <div class="min-w-0">
        <p class="text-sm text-ink">{t("settings.panelOpacityLabel")}</p>
        <p class="text-xs text-ink-faint">{t("settings.panelOpacityHint")}</p>
      </div>
      <label class="flex shrink-0 items-center gap-3 text-xs text-ink-muted">
        <input
          type="range"
          min={OPACITY_MIN}
          max={OPACITY_MAX}
          step="5"
          class="w-40 accent-[var(--color-accent)]"
          aria-label={t("settings.panelOpacityLabel")}
          value={panelOpacity.panel}
          oninput={(e) => panelOpacity.setPanel(e.currentTarget.valueAsNumber)}
        />
        <span class="w-10 text-right tabular-nums">{panelOpacity.panel}%</span>
      </label>
    </div>

    {#if backdrop.value === "light"}
    <div class="mt-4 flex flex-wrap items-center justify-between gap-4 border-t border-surface-2 pt-4">
      <div class="min-w-0">
        <p class="text-sm text-ink">{t("settings.glowLabel")}</p>
        <p class="text-xs text-ink-faint">{t("settings.glowHint")}</p>
      </div>
      <div class="flex shrink-0 gap-1" role="radiogroup" aria-label={t("settings.glowLabel")}>
        {#each GLOW_VARIANTS as id (id)}
          <button
            type="button"
            role="radio"
            aria-checked={glow.variant === id}
            class="rounded-lg px-3 py-1 text-xs font-medium transition-colors {glow.variant === id
              ? 'bg-accent-soft text-ink'
              : 'border border-surface-3 text-ink-muted hover:bg-surface-2'}"
            onclick={() => glow.set(id)}
          >
            {t(`settings.glow.${id}` as TranslationKey)}
          </button>
        {/each}
      </div>
    </div>
    {/if}

    <div class="mt-4 flex flex-wrap items-center justify-between gap-4 border-t border-surface-2 pt-4">
      <div class="min-w-0">
        <p class="text-sm text-ink">{t("settings.languageLabel")}</p>
        <p class="text-xs text-ink-faint">{t("settings.languageHint")}</p>
      </div>
      <div class="flex shrink-0 gap-1" role="radiogroup" aria-label={t("settings.languageLabel")}>
        {#each LANGUAGE_CHOICES as choice (choice.id)}
          <button
            type="button"
            role="radio"
            aria-checked={locale.current === choice.id}
            lang={choice.id}
            class="rounded-lg px-3 py-1 text-xs font-medium transition-colors {locale.current === choice.id
              ? 'bg-accent-soft text-ink'
              : 'border border-surface-3 text-ink-muted hover:bg-surface-2'}"
            onclick={() => locale.set(choice.id)}
          >
            {choice.label}
          </button>
        {/each}
      </div>
    </div>
  </section>

  <section class="flex flex-col gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6">
    <div>
      <h2 class="text-sm font-medium">{t("settings.sidebarTitle")}</h2>
      <p class="mt-1 text-xs leading-relaxed text-ink-muted">
        {t("settings.sidebarSubtitle")}
      </p>
    </div>

    <ul class="grid gap-x-6 gap-y-2 sm:grid-cols-2">
      {#each moduleStore.modules as module (module.id)}
        <li class="flex items-center gap-3">
          <input
            id="sidebar-{module.id}"
            type="checkbox"
            class="size-4 shrink-0 accent-[var(--color-accent)]"
            checked={!hiddenModules.isHidden(module.id)}
            onchange={() => hiddenModules.toggle(module.id)}
          />
          <label for="sidebar-{module.id}" class="text-sm text-ink">
            {tModuleName(module.id, module.name)}
          </label>
        </li>
      {/each}
    </ul>
  </section>

  <section class="flex flex-col gap-3 rounded-xl border border-surface-2 bg-surface-1 p-6">
    <h2 class="text-sm font-medium">{t("about.title")}</h2>
    <p class="text-xs leading-relaxed text-ink-muted">{t("about.official")}</p>
    <h3 class="mt-1 text-sm text-ink">{t("about.neverTitle")}</h3>
    <ul class="flex list-disc flex-col gap-1.5 pl-5 text-xs leading-relaxed text-ink-muted">
      <li>{t("about.never.network")}</li>
      <li>{t("about.never.update")}</li>
      <li>{t("about.never.service")}</li>
      <li>{t("about.never.delete")}</li>
    </ul>
    <p class="text-xs text-ink-faint">{t("about.verify")}</p>
  </section>

  {#if store.error}
    <p
      class="rounded-xl border border-risk-danger/30 bg-surface-1 p-4 text-sm text-risk-danger"
    >
      {store.error}
    </p>
  {/if}

  {#if store.status === "loading" || store.settings === null}
    <LoadingNote text={t("settings.loading")} />
  {:else}
    {@const settings = store.settings}

    <section class="flex flex-col gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6">
      <div>
        <h2 class="text-sm font-medium">{t("settings.packsTitle")}</h2>
        <p class="mt-1 text-xs leading-relaxed text-ink-muted">
          {t("settings.packsSubtitle")}
        </p>
      </div>

      <ul class="flex flex-col divide-y divide-surface-2">
        {#each settings.cleaner.packs as pack (pack.id)}
          <li class="flex items-start gap-3 py-3">
            <input
              id="pack-{pack.id}"
              type="checkbox"
              class="mt-1 size-4 shrink-0 accent-[var(--color-accent)]"
              checked={pack.enabled}
              onchange={(event) =>
                store.setPackEnabled(pack.id, event.currentTarget.checked)}
            />
            <span class="min-w-0 flex-1">
              <label
                for="pack-{pack.id}"
                class="block text-sm font-medium text-ink"
              >
                {pack.name}
              </label>
              <span class="mt-1 block text-xs leading-relaxed text-ink-muted">
                {pack.description}
              </span>
              <span class="mt-1 block text-[0.7rem] text-ink-faint">
                {t("settings.packRules", { count: pack.ruleCount })} ·
                {pack.enabledByDefault
                  ? t("settings.packDefaultOn")
                  : t("settings.packDefaultOff")}
              </span>
            </span>
          </li>
        {/each}
      </ul>
    </section>

    <section class="flex flex-col gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6">
      <h2 class="text-sm font-medium">{t("settings.duplicatesTitle")}</h2>

      <label class="flex flex-col gap-1">
        <span class="text-sm">{t("settings.minSize")}</span>
        <select
          class="max-w-48 rounded-lg border border-surface-2 bg-surface-1 px-2 py-1.5 text-sm"
          value={settings.duplicates.minSizeBytes}
          onchange={(event) =>
            store.setMinSizeBytes(Number(event.currentTarget.value))}
        >
          {#each MIN_SIZE_CHOICES as choice (choice.value)}
            <option value={choice.value}>{choice.label}</option>
          {/each}
        </select>
        <span class="text-xs text-ink-faint">
          {t("settings.minSizeHint", {
            size: formatBytes(MIN_DUPLICATE_BYTES),
          })}
        </span>
      </label>
    </section>

    <section class="flex flex-col gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6">
      <h2 class="text-sm font-medium">{t("settings.quarantineTitle")}</h2>

      <label class="flex flex-col gap-1">
        <span class="text-sm">{t("settings.retentionDays")}</span>
        <span class="flex items-center gap-2">
          <input
            type="number"
            class="w-24 rounded-lg border border-surface-2 bg-surface-1 px-2 py-1.5 text-sm tabular-nums"
            min={RETENTION_MIN}
            max={RETENTION_MAX}
            value={settings.quarantine.retentionDays}
            onchange={(event) =>
              store.setRetentionDays(Number(event.currentTarget.value))}
          />
          <span class="text-sm text-ink-muted">{t("settings.days")}</span>
        </span>
        <span class="text-xs leading-relaxed text-ink-faint">
          {t("settings.retentionHint", {
            min: RETENTION_MIN,
            max: RETENTION_MAX,
          })}
        </span>
      </label>

      <label class="flex flex-col gap-1">
        <span class="text-sm">{t("settings.maxBytes")}</span>
        <select
          class="max-w-48 rounded-lg border border-surface-2 bg-surface-1 px-2 py-1.5 text-sm"
          value={settings.quarantine.maxBytes}
          onchange={(event) =>
            store.setMaxBytes(Number(event.currentTarget.value))}
        >
          {#each CAP_CHOICES as choice (choice.value)}
            <option value={choice.value}>{choice.label}</option>
          {/each}
          {#if !CAP_CHOICES.some((choice) => choice.value === settings.quarantine.maxBytes)}
            <option value={settings.quarantine.maxBytes}>
              {formatBytes(settings.quarantine.maxBytes)}
            </option>
          {/if}
        </select>
        <span class="text-xs leading-relaxed text-ink-faint">
          {t("settings.maxBytesHint")}
        </span>
      </label>
    </section>

    <div class="flex items-center gap-3">
      <button
        type="button"
        class="btn-primary"
        disabled={store.status === "saving"}
        onclick={() => store.save()}
      >
        {store.status === "saving" ? t("settings.saving") : t("settings.save")}
      </button>

      {#if store.adjusted}
        <p class="text-xs text-risk-review">{t("settings.adjusted")}</p>
      {:else if store.saved}
        <p class="text-xs text-risk-safe">{t("settings.saved")}</p>
      {/if}
    </div>

    <section class="flex flex-col gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6">
      <div>
        <h2 class="text-sm font-medium">{t("settings.profileTitle")}</h2>
        <p class="mt-1 text-xs leading-relaxed text-ink-muted">
          {t("settings.profileSubtitle")}
        </p>
      </div>

      <div class="flex flex-wrap gap-2">
        <button type="button" class="btn-secondary" disabled={profileBusy} onclick={exportProfile}>
          {t("settings.profileExport")}
        </button>
        <button type="button" class="btn-secondary" disabled={profileBusy} onclick={importProfile}>
          {t("settings.profileImport")}
        </button>
      </div>

      {#if preview}
        {@const checked = preview.profile}
        {@const pending = checked.recommendations.length}
        <div class="flex flex-col gap-3 rounded-lg border border-surface-2 p-4">
          <h3 class="text-sm text-ink">{t("settings.profileTweaksHeading")}</h3>
          {#if checked.tweaks.length === 0}
            <p class="text-xs text-ink-faint">{t("settings.profileNoTweaks")}</p>
          {:else}
            <ul class="flex flex-col gap-1.5 text-xs">
              {#each checked.tweaks as tweak (tweak.id)}
                <li class="flex flex-col">
                  <span class="text-ink">{tweak.name}</span>
                  <span class="text-ink-faint">
                    {#if tweak.error}
                      {tweak.error}
                    {:else if tweak.applied}
                      {t("settings.profileTweakAlready")}
                    {:else}
                      {t("settings.profileTweakChange", {
                        from: tweak.current === null ? t("settings.profileNotSet") : tweak.current,
                        to: tweak.target,
                      })}
                    {/if}
                  </span>
                  <span class="select-text break-all text-[0.7rem] text-ink-faint">{tweak.path}</span>
                </li>
              {/each}
            </ul>
          {/if}

          <h3 class="text-sm text-ink">{t("settings.profileSettingsHeading")}</h3>
          {#if preview.changes.length === 0}
            <p class="text-xs text-ink-faint">{t("settings.profileNoSettingChanges")}</p>
          {:else}
            <ul class="flex list-disc flex-col gap-1 pl-5 text-xs text-ink-muted">
              {#each preview.changes as change (change)}
                <li>{change}</li>
              {/each}
            </ul>
          {/if}

          <div class="flex gap-2">
            <button
              type="button"
              class="btn-primary"
              disabled={profileBusy || (pending === 0 && preview.changes.length === 0)}
              onclick={() => (confirming = true)}
            >
              {t("settings.profileApply")}
            </button>
            <button
              type="button"
              class="btn-secondary"
              disabled={profileBusy}
              onclick={() => (preview = null)}
            >
              {t("settings.profileCancel")}
            </button>
          </div>
        </div>
      {/if}

      {#if profileNote}
        <p class="select-text break-all text-xs text-risk-safe">{profileNote}</p>
      {/if}
      {#if profileError}
        <p class="text-xs text-risk-danger">{profileError}</p>
      {/if}
      {#each profileFailures as failure (failure)}
        <p class="select-text break-all text-xs text-risk-review">{failure}</p>
      {/each}
    </section>
  {/if}
</div>

<ConfirmDialog
  open={confirming}
  title={t("settings.profileConfirmTitle")}
  message={t("settings.profileConfirmBody", {
    tweaks: preview?.profile.recommendations.length ?? 0,
    settings: preview?.changes.length ?? 0,
  })}
  note={t("settings.profileConfirmNote")}
  confirmLabel={t("settings.profileApply")}
  onconfirm={applyProfile}
  oncancel={() => (confirming = false)}
/>
