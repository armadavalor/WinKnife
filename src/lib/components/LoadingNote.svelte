<script lang="ts">
  import KnifeLogo from "$lib/components/KnifeLogo.svelte";

  // A loading line with the folding knife in front of it. The knife stays
  // still when animations are off; the text carries the meaning either way.
  let { text, class: className = "" }: { text: string; class?: string } = $props();
</script>

<p class={`flex items-center gap-2 text-sm text-ink-faint ${className}`} role="status">
  <KnifeLogo size={18} loading />
  <span>{text}</span>
</p>
