<!--
  Determinate or indeterminate progress.

  A scan cannot know its own total up front (that would mean walking twice), so
  the indeterminate variant is the common one; `pct` is used where a total is
  genuinely known, like an execute batch.
-->
<script lang="ts">
  interface Props {
    /** 0-100 when known; omit for an indeterminate bar. */
    pct?: number | null;
    /** Accessible label describing what is in progress. */
    label: string;
  }

  let { pct = null, label }: Props = $props();

  const clamped = $derived(
    pct === null ? null : Math.min(100, Math.max(0, pct)),
  );
</script>

<div
  class="h-1 w-full overflow-hidden rounded-full bg-surface-2"
  role="progressbar"
  aria-label={label}
  aria-valuemin={clamped === null ? undefined : 0}
  aria-valuemax={clamped === null ? undefined : 100}
  aria-valuenow={clamped ?? undefined}
>
  {#if clamped === null}
    <div class="h-full w-1/3 animate-pulse rounded-full bg-accent"></div>
  {:else}
    <div
      class="h-full rounded-full bg-accent transition-[width] duration-200"
      style="width: {clamped}%"
    ></div>
  {/if}
</div>
