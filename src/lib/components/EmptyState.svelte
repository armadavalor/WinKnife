<!--
  Centered "nothing here" block: icon, one sentence, optional next action.
-->
<script lang="ts">
  import Icon from "$lib/components/Icon.svelte";
  import type { Snippet } from "svelte";

  interface Props {
    message: string;
    icon?: Snippet;
    action?: Snippet;
  }

  let { message, icon, action }: Props = $props();
</script>

<div
  class="flex flex-col items-center gap-3 rounded-xl border border-dashed border-surface-3 px-5 py-8 text-center"
>
  <span class="flex size-8 items-center justify-center text-ink-faint">
    {#if icon}
      {@render icon()}
    {:else}
      <Icon name="folder" class="size-8" />
    {/if}
  </span>
  <p class="text-sm text-ink-faint">{message}</p>
  {#if action}
    {@render action()}
  {/if}
</div>
