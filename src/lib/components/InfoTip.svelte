<!--
  Small (i) button with a popover. Opens on hover, focus and click; closes on
  mouse leave, blur and Escape. The text is also the button's aria-label, so
  nothing hidden here is hidden from assistive tech.
-->
<script lang="ts">
  interface Props {
    /** The text shown in the popover. */
    text: string;
  }

  let { text }: Props = $props();

  let hovered = $state(false);
  let focused = $state(false);
  let pinned = $state(false);

  const open = $derived(hovered || focused || pinned);

  function close(): void {
    hovered = false;
    focused = false;
    pinned = false;
  }

  function onkeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") close();
  }
</script>

<span
  class="relative inline-flex align-middle"
  role="presentation"
  onmouseenter={() => (hovered = true)}
  onmouseleave={() => {
    hovered = false;
    pinned = false;
  }}
>
  <button
    type="button"
    class="inline-flex size-4 shrink-0 items-center justify-center rounded-full text-ink-faint transition-colors hover:text-ink focus-visible:text-ink"
    aria-label={text}
    aria-expanded={open}
    onclick={() => (pinned = !pinned)}
    onfocus={() => (focused = true)}
    onblur={() => {
      focused = false;
      pinned = false;
    }}
    {onkeydown}
  >
    <svg
      class="size-4"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v5" />
      <path d="M12 8h.01" />
    </svg>
  </button>
  {#if open}
    <span
      role="tooltip"
      class="absolute left-0 top-full z-20 mt-1.5 w-max max-w-sm rounded-lg border border-surface-2 bg-surface-1 p-3 text-left text-xs font-normal normal-case leading-relaxed tracking-normal text-ink-muted shadow-lg"
    >
      {text}
    </span>
  {/if}
</span>
