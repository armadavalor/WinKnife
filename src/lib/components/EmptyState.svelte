<!--
  Centered "nothing here" block: icon, one sentence, optional next action.
-->
<script lang="ts">
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
      <svg
        class="size-8"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
      </svg>
    {/if}
  </span>
  <p class="text-sm text-ink-faint">{message}</p>
  {#if action}
    {@render action()}
  {/if}
</div>
