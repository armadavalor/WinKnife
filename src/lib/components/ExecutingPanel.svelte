<!--
  The "executing" step shared by the routes that move a selection to
  quarantine: how far the batch is, what it is moving, and a Stop button
  (issue #39).
-->
<script lang="ts">
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import { truncatePath } from "$lib/format";
  import { t } from "$lib/i18n";
  import type { ExecTracker } from "$lib/stores/execProgress.svelte";

  interface Props {
    tracker: ExecTracker;
  }

  let { tracker }: Props = $props();
</script>

<section class="flex flex-col gap-3 rounded-xl border border-surface-2 bg-surface-1 p-6">
  <div class="flex items-center gap-3">
    <p class="flex-1 text-sm">{t("cleaner.executing")}</p>
    <button
      type="button"
      class="btn-ghost px-3! py-1.5!"
      disabled={tracker.stopping}
      onclick={() => tracker.stop()}
    >
      {tracker.stopping ? t("cleaner.stopping") : t("cleaner.stop")}
    </button>
  </div>
  <ProgressBar label={t("cleaner.executing")} pct={tracker.pct} />
  <p class="font-mono text-xs text-ink-faint">
    {tracker.currentPath ? truncatePath(tracker.currentPath, 70) : "…"}
  </p>
  {#if tracker.total !== null}
    <p class="text-xs text-ink-faint">
      {t("exec.progress", { done: tracker.done, total: tracker.total })}
    </p>
  {/if}
  {#if tracker.stopping}
    <p class="text-xs text-ink-faint">{t("exec.stoppingNote")}</p>
  {/if}
</section>
