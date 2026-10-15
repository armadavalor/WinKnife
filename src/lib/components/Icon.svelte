<!--
  One inline SVG icon from the vendored Lucide set (`$lib/icons`). Every icon
  shares the same 24px grid, 1.75 stroke and round joins, so they read as one
  family at sidebar and card sizes.
-->
<script lang="ts">
  import { ICONS, type IconName } from "$lib/icons";

  interface Props {
    name: IconName;
    /** Extra classes for sizing/colour. */
    class?: string;
    style?: string;
  }

  let { name, class: className = "size-5", style }: Props = $props();
</script>

<svg
  class={className}
  {style}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.75"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#each ICONS[name] as [tag, attrs], i (i)}
    <svelte:element this={tag} {...attrs} />
  {/each}
</svg>
