<!--
  Page title with the former subtitle tucked into an info popover.
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  import InfoTip from "$lib/components/InfoTip.svelte";

  interface Props {
    title: string;
    /** The old subtitle; reachable through the (i) button. */
    info?: string;
    /** Right-aligned controls. */
    actions?: Snippet;
  }

  let { title, info, actions }: Props = $props();
</script>

<header class="flex items-center justify-between gap-4">
  <div class="flex items-center gap-2">
    <h1 class="text-2xl font-semibold tracking-tight">{title}</h1>
    {#if info}
      <InfoTip text={info} />
    {/if}
  </div>
  {#if actions}
    <div class="flex items-center gap-2">{@render actions()}</div>
  {/if}
</header>
