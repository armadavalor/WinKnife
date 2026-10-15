<!--
  Settings: per-module storage, rendered from what Rust says it has.

  The pack list is not hard-coded — it is whatever loaded on this machine, so a
  community pack dropped into `rules/` appears here with no UI change. And the
  numeric fields show what was *stored*, which is not always what was typed:
  Rust clamps retention and the duplicate size floor, and the page says so.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import PageHeader from "$lib/components/PageHeader.svelte";
  import { formatBytes } from "$lib/format";
  import { t, type TranslationKey } from "$lib/i18n";
  import { settingsStore } from "$lib/stores/settings.svelte";
  import { THEMES, theme } from "$lib/stores/theme.svelte";

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
  </section>

  {#if store.error}
    <p
      class="rounded-xl border border-risk-danger/30 bg-surface-1 p-4 text-sm text-risk-danger"
    >
      {store.error}
    </p>
  {/if}

  {#if store.status === "loading" || store.settings === null}
    <p class="text-sm text-ink-faint">{t("settings.loading")}</p>
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
  {/if}
</div>
