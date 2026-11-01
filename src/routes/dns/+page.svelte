<!--
  DNS switcher (#16): list adapters and their DNS, switch to a preset, undo.
  Reading is free; changing needs administrator rights (the shared
  needs-admin flow handles the restart).
-->
<script lang="ts">
  import { onMount } from "svelte";

  import { dnsApply, dnsRevert, dnsState } from "$lib/api";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import LoadingNote from "$lib/components/LoadingNote.svelte";
  import PageHeader from "$lib/components/PageHeader.svelte";
  import { t, tModuleDescription, tModuleName } from "$lib/i18n";
  import { moduleStore } from "$lib/stores/modules.svelte";
  import { isApiError, type DnsChange, type DnsPreset, type DnsState } from "$lib/types";

  const MODULE_ID = "dns";
  const PRESETS: DnsPreset[] = ["cloudflare", "google", "quad9", "automatic"];

  const manifest = $derived(moduleStore.find(MODULE_ID));

  let dns = $state<DnsState | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let report = $state<string | null>(null);

  function message(err: unknown): string {
    return isApiError(err) ? err.message : String(err);
  }

  const describe = (servers: string[], automatic: boolean): string =>
    automatic || servers.length === 0 ? t("dns.automatic") : servers.join(", ");

  const previousOf = (c: DnsChange): string => describe(c.previous, c.previousAutomatic);

  /** Newest history entry for an adapter, which is what Undo restores. */
  const lastChange = (index: number): DnsChange | undefined =>
    dns?.history.findLast((c) => c.index === index);

  async function load(): Promise<void> {
    busy = true;
    error = null;
    try {
      dns = await dnsState();
    } catch (err: unknown) {
      error = message(err);
    } finally {
      busy = false;
    }
  }

  async function run(action: () => Promise<string>): Promise<void> {
    busy = true;
    report = null;
    try {
      report = await action();
    } catch (err: unknown) {
      report = t("dns.failed", { error: message(err) });
    }
    busy = false;
    await load();
  }

  const apply = (index: number, preset: DnsPreset): Promise<void> =>
    run(async () => {
      const c = await dnsApply(index, preset);
      return t("dns.applied", {
        name: c.name,
        preset: t(`dns.preset.${preset}`),
        previous: previousOf(c),
      });
    });

  const revert = (index: number): Promise<void> =>
    run(async () => {
      const c = await dnsRevert(index);
      return t("dns.reverted", { name: c.name, previous: previousOf(c) });
    });

  onMount(() => {
    void load();
  });
</script>

<div class="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
  <PageHeader
    title={manifest ? tModuleName(manifest.id, manifest.name) : MODULE_ID}
    info={[manifest ? tModuleDescription(manifest.id, manifest.description) : "", t("dns.intro")]
      .filter(Boolean)
      .join(" ")}
  />

  {#if error}
    <p class="rounded-xl border border-risk-danger/30 bg-surface-1 p-4 text-sm text-risk-danger">
      {t("dns.loadError")} {error}
    </p>
  {/if}

  {#if !dns}
    {#if !error}<LoadingNote text={t("dns.loading")} />{/if}
  {:else}
    <div class="flex flex-col gap-3">
      <div class="flex flex-wrap items-center gap-2">
        {#if report}<p class="min-w-0 flex-1 text-sm text-ink-muted">{report}</p>{:else}<span class="flex-1"></span>{/if}
        <button type="button" class="btn-outline px-3! py-1.5! text-xs" disabled={busy} onclick={load}>
          {t("dns.refresh")}
        </button>
      </div>

      {#if dns.adapters.length === 0}
        <EmptyState message={t("dns.empty")} />
      {:else}
        <ul class="flex flex-col gap-2">
          {#each dns.adapters as adapter (adapter.index)}
            {@const last = lastChange(adapter.index)}
            <li class="flex flex-col gap-3 rounded-xl border border-surface-2 bg-surface-1 p-4">
              <div class="min-w-0">
                <p class="font-medium">{adapter.name}</p>
                <p class="mt-1 select-text font-mono text-xs text-ink-muted">
                  {t("dns.current", { servers: describe(adapter.servers, adapter.automatic) })}
                </p>
              </div>
              <div class="flex flex-wrap items-center gap-2">
                {#each PRESETS as preset (preset)}
                  <button
                    type="button"
                    class="btn-outline px-3! py-1.5! text-xs"
                    disabled={busy}
                    onclick={() => apply(adapter.index, preset)}
                  >
                    {t(`dns.preset.${preset}`)}
                  </button>
                {/each}
                {#if last}
                  <button
                    type="button"
                    class="btn-ghost px-3! py-1.5! text-xs"
                    disabled={busy}
                    title={previousOf(last)}
                    onclick={() => revert(adapter.index)}
                  >
                    {t("dns.revert")}
                  </button>
                {/if}
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
</div>
