<!--
  Maps the `icon` name from a module manifest to an inline SVG.

  Inline rather than an icon font or sprite sheet: the app must work with no
  network and the icon set is small. Unknown names fall back to a neutral
  glyph so a new module is never invisible in the sidebar.
-->
<script lang="ts">
  interface Props {
    /** Icon name from `ModuleManifest.icon`. */
    name: string;
    /** Extra classes for sizing/colour. */
    class?: string;
  }

  let { name, class: className = "size-5" }: Props = $props();

  const paths: Record<string, string> = {
    dashboard:
      "M4 4h6v8H4zM4 16h6v4H4zM14 4h6v4h-6zM14 12h6v8h-6z",
    beaker:
      "M9 3v6.5L4.6 17A2 2 0 0 0 6.3 20h11.4a2 2 0 0 0 1.7-3L15 9.5V3M8 3h8M7.5 14h9",
    broom: "M4 20l6-6m0 0 4-4m-4 4 4 4m2-10 4-4M9 15l6 6",
    power: "M12 3v9M7.5 6.4a8 8 0 1 0 9 0",
    shield: "M12 3l7 3v6c0 4.2-2.8 7.4-7 9-4.2-1.6-7-4.8-7-9V6z",
    file: "M6 3h8l4 4v14H6zM14 3v4h4",
  };
</script>

<svg
  class={className}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.6"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d={paths[name] ?? paths.file} />
</svg>
