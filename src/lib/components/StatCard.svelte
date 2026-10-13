<!--
  A single headline number with an optional info popover and footer.
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  import InfoTip from "$lib/components/InfoTip.svelte";

  interface Props {
    label: string;
    value: string;
    info?: string;
    footer?: Snippet;
  }

  let { label, value, info, footer }: Props = $props();
</script>

<div class="rounded-xl border border-surface-2 bg-surface-1 p-4">
  <p class="flex items-center gap-1.5 text-xs uppercase tracking-wider text-ink-faint">
    <span>{label}</span>
    {#if info}
      <InfoTip text={info} />
    {/if}
  </p>
  <p class="mt-1 text-xl font-semibold tabular-nums">{value}</p>
  {#if footer}
    {@render footer()}
  {/if}
</div>
