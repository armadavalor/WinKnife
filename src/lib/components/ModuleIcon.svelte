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
    copies: "M9 9h11v11H9zM4 4h11v3M4 4v11h3",
    settings:
      "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6M12 2v3M12 19v3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M2 12h3M19 12h3M4.9 19.1L7 17M17 7l2.1-2.1",
    file: "M6 3h8l4 4v14H6zM14 3v4h4",
    treemap: "M3 4h18v16H3zM3 12h11M14 4v16M14 8h7M3 16h11",
    rocket:
      "M5 15l-2 6 6-2M9 19a7 7 0 0 1-4-4l7-7a9 9 0 0 1 8-3 9 9 0 0 1-3 8zM14.5 9.5h.01",
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
