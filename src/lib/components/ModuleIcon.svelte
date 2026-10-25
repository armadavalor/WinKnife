<!--
  Maps the `icon` name from a module manifest to a glyph in the shared icon set.

  Unknown names fall back to a neutral glyph so a new module is never invisible
  in the sidebar.
-->
<script lang="ts">
  import Icon from "$lib/components/Icon.svelte";
  import type { IconName } from "$lib/icons";

  interface Props {
    /** Icon name from `ModuleManifest.icon`. */
    name: string;
    /** Extra classes for sizing/colour. */
    class?: string;
  }

  let { name, class: className = "size-5" }: Props = $props();

  const MANIFEST_ICONS: Record<string, IconName> = {
    dashboard: "layout-dashboard",
    beaker: "flask-conical",
    broom: "brush-cleaning",
    power: "power",
    coffee: "coffee",
    folder: "folder-search",
    shield: "shield-check",
    copies: "copy",
    settings: "settings",
    file: "file",
    treemap: "chart-pie",
    unlock: "lock-open",
    rocket: "rocket",
    panels: "app-window",
    restore: "history",
    hosts: "globe",
    menu: "list",
    network: "globe",
  };
</script>

<Icon name={MANIFEST_ICONS[name] ?? "file"} class={className} />
