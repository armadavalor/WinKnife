<!--
  Determinate or indeterminate progress.

  A scan cannot know its own total up front (that would mean walking twice), so
  the indeterminate variant is the common one; `pct` is used where a total is
  genuinely known, like an execute batch.

  Running bars always carry a little motion so slow work does not read as
  frozen (Polish item 5): a sliding segment when the total is unknown, a
  drifting stripe when it is known. The motion lives in app.css and stops under
  `data-animations="off"` on <html>.
-->
<script lang="ts">
  import { clampPct, progressMode } from "$lib/progress/mode";

  interface Props {
    /** 0-100 when known; omit for an indeterminate bar. */
    pct?: number | null;
    /** Accessible label describing what is in progress. */
    label: string;
    /** False for gauges that are not work in progress (disk capacity). */
    active?: boolean;
  }

  let { pct = null, label, active = true }: Props = $props();

  const clamped = $derived(clampPct(pct));
  const mode = $derived(progressMode(pct, active));
</script>

<div
  class="h-1 w-full overflow-hidden rounded-full bg-surface-2"
  role="progressbar"
  aria-label={label}
  aria-valuemin={clamped === null ? undefined : 0}
  aria-valuemax={clamped === null ? undefined : 100}
  aria-valuenow={clamped ?? undefined}
  data-mode={mode}
>
  {#if clamped === null}
    <div class="progress-slide h-full w-1/3 rounded-full bg-accent"></div>
  {:else}
    <div
      class="h-full rounded-full bg-accent transition-[width] duration-200"
      class:progress-stripe={mode === "running"}
      style="width: {clamped}%"
    ></div>
  {/if}
</div>
