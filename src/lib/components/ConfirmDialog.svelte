<!--
  The last thing between a user and a change to their machine.

  It always states what will happen and where the files go — ARCHITECTURE
  section 7 requires the "goes to quarantine, reversible for 7 days" line, and
  the cleaner passes an extra warning for the recycle bin, which is the one
  action that is not reversible.
-->
<script lang="ts">
  import { t } from "$lib/i18n";

  interface Props {
    /** Whether the dialog is showing. */
    open: boolean;
    /** Dialog heading. */
    title: string;
    /** Main sentence: what is about to happen. */
    message: string;
    /** Reassurance line, e.g. the quarantine promise. */
    note?: string;
    /** A path or similar the user may need to copy, shown selectable. */
    detail?: string;
    /** Extra warning shown in the danger colour. */
    warning?: string;
    /** Label of the confirming button. */
    confirmLabel: string;
    /** Called when the user confirms. */
    onconfirm: () => void;
    /** Called when the user backs out. */
    oncancel: () => void;
  }

  let {
    open,
    title,
    message,
    note,
    detail,
    warning,
    confirmLabel,
    onconfirm,
    oncancel,
  }: Props = $props();

  let dialog = $state<HTMLDivElement | null>(null);

  // Focus lands on the dialog when it opens, so Escape and Tab behave.
  $effect(() => {
    if (open) {
      dialog?.focus();
    }
  });

  function onkeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      oncancel();
    }
  }
</script>

{#if open}
  <div
    class="fixed inset-0 z-40 flex items-center justify-center bg-scrim p-6"
  >
    <div
      bind:this={dialog}
      class="w-full max-w-md rounded-xl border border-surface-3 bg-surface-1 p-6 shadow-2xl outline-none"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="confirm-title"
      tabindex="-1"
      {onkeydown}
    >
      <h2 id="confirm-title" class="text-lg font-semibold tracking-tight">
        {title}
      </h2>

      <p class="mt-3 text-sm leading-relaxed text-ink-muted">{message}</p>

      {#if note}
        <p class="mt-3 rounded-lg bg-surface-2 p-3 text-xs leading-relaxed text-ink-muted">
          {note}
        </p>
      {/if}

      {#if detail}
        <p class="mt-2 select-text break-all font-mono text-xs text-ink-muted">
          {detail}
        </p>
      {/if}

      {#if warning}
        <p
          class="mt-3 rounded-lg border border-risk-danger/30 p-3 text-xs leading-relaxed text-risk-danger"
        >
          {warning}
        </p>
      {/if}

      <div class="mt-6 flex justify-end gap-2">
        <button
          type="button"
          class="btn-ghost px-3! py-1.5!"
          onclick={oncancel}
        >
          {t("action.cancel")}
        </button>
        <button
          type="button"
          class="btn-primary px-3! py-1.5!"
          onclick={onconfirm}
        >
          {confirmLabel}
        </button>
      </div>
    </div>
  </div>
{/if}
