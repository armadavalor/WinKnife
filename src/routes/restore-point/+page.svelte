<!--
  Restore point route (module spec 09).

  Read-only status, one button, one plain sentence per result. WinKnife never
  changes System Protection: when it is off, the page offers Windows' own
  dialog, and only opens it when the user clicks.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import { restorePointCreate, restorePointOpenSettings, restorePointStatus } from "$lib/api";
  import InfoTip from "$lib/components/InfoTip.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { isApiError, type CreateOutcome, type RestorePointStatus } from "$lib/types";

  const MODULE_ID = "restore-point";
  /** Mirrors the backend default; the backend sanitizes again regardless. */
  const FALLBACK_MAX_CHARS = 64;

  const manifest = $derived(moduleStore.find(MODULE_ID));

  let status = $state<RestorePointStatus | null>(null);
  let description = $state("");
  let creating = $state(false);
  let outcome = $state<CreateOutcome | null>(null);
  let error = $state<string | null>(null);

  const maxChars = $derived(status?.maxDescriptionChars ?? FALLBACK_MAX_CHARS);
  const offerSettings = $derived(
    outcome?.state === "protectionOff" || status?.protection === "off",
  );

  function message(err: unknown): string {
    return isApiError(err) ? err.message : String(err);
  }

  async function loadStatus(): Promise<void> {
    try {
      status = await restorePointStatus();
    } catch (err: unknown) {
      error = message(err);
    }
  }

  onMount(() => {
    void loadStatus();
  });

  async function create(): Promise<void> {
    creating = true;
    outcome = null;
    error = null;
    try {
      outcome = await restorePointCreate(description);
    } catch (err: unknown) {
      error = message(err);
    } finally {
      creating = false;
    }
    void loadStatus();
  }

  async function openSettings(): Promise<void> {
    error = null;
    try {
      await restorePointOpenSettings();
    } catch (err: unknown) {
      error = message(err);
    }
  }

  function outcomeText(result: CreateOutcome): string {
    if (result.state === "failed") {
      return t("restorePoint.outcome.failed", { code: result.code });
    }
    return t(`restorePoint.outcome.${result.state}`);
  }

  function outcomeTone(result: CreateOutcome): string {
    switch (result.state) {
      case "created":
        return "border-risk-safe/30 text-risk-safe";
      case "failed":
      case "timedOut":
        return "border-risk-danger/30 text-risk-danger";
      default:
        return "border-risk-review/30 text-risk-review";
    }
  }
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(manifest.id, manifest.name) : MODULE_ID}
    info={manifest ? tModuleDescription(manifest.id, manifest.description) : undefined}
  />

  {#if error}
    <p class="rounded-xl border border-risk-danger/30 p-4 text-sm text-risk-danger">
      {error}
    </p>
  {/if}

  <section class="flex flex-col gap-2 rounded-xl border border-surface-2 bg-surface-1 p-6">
    <span class="text-xs font-medium uppercase tracking-wider text-ink-faint">
      {t("restorePoint.statusLabel")}
    </span>
    {#if status}
      <p class="text-sm font-medium">{t(`restorePoint.protection.${status.protection}`)}</p>
      <p class="text-xs leading-relaxed text-ink-muted">{status.reason}</p>
    {:else}
      <p class="text-sm text-ink-faint">{t("restorePoint.statusLoading")}</p>
    {/if}
  </section>

  <section class="flex flex-col gap-4 rounded-xl border border-surface-2 bg-surface-1 p-6">
    <div class="flex flex-col gap-2">
      <label
        class="text-xs font-medium uppercase tracking-wider text-ink-faint"
        for="restore-description"
      >
        {t("restorePoint.descriptionLabel")}
      </label>
      <input
        id="restore-description"
        type="text"
        maxlength={maxChars}
        placeholder={status?.defaultDescription ?? ""}
        bind:value={description}
        disabled={creating}
        class="rounded-lg border border-surface-3 bg-surface-0 px-3 py-1.5 text-sm text-ink"
      />
      <p class="text-xs text-ink-faint">
        {t("restorePoint.descriptionHint", { max: maxChars })}
      </p>
    </div>

    <div class="flex items-center gap-2">
      <button
        type="button"
        class="rounded-lg bg-accent-soft px-4 py-2 text-sm font-medium text-ink transition-colors hover:bg-accent hover:text-surface-0 disabled:cursor-not-allowed disabled:opacity-40"
        disabled={creating}
        onclick={create}
      >
        {creating ? t("restorePoint.creating") : t("restorePoint.create")}
      </button>
      <InfoTip text={t("restorePoint.uacNotice")} />
    </div>

    {#if outcome}
      <p class="rounded-lg border p-3 text-sm leading-relaxed {outcomeTone(outcome)}" role="status">
        {outcomeText(outcome)}
      </p>
    {/if}

    {#if offerSettings}
      <div class="flex flex-col items-start gap-2">
        <p class="text-xs leading-relaxed text-ink-muted">{t("restorePoint.settingsHint")}</p>
        <button
          type="button"
          class="rounded-lg border border-surface-3 px-3 py-1.5 text-sm transition-colors hover:bg-surface-2"
          onclick={openSettings}
        >
          {t("restorePoint.openSettings")}
        </button>
      </div>
    {/if}
  </section>
</div>
