/**
 * Dialog focus handling as a Svelte action: focus moves into the node when it
 * mounts, Escape calls `onescape`, and focus returns to whatever held it
 * before when the node goes away. With `trap` (modal dialogs) Tab and
 * Shift+Tab also wrap inside the node instead of leaking to the page behind.
 */
export interface FocusTrapOptions {
  onescape: () => void;
  trap?: boolean;
}

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function focusTrap(node: HTMLElement, options: FocusTrapOptions) {
  let opts = options;
  const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  node.focus();

  function onkeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      opts.onescape();
      return;
    }
    if (event.key !== "Tab" || !opts.trap) return;
    const items = [...node.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
      (el) => el.offsetParent !== null,
    );
    if (items.length === 0) {
      event.preventDefault();
      return;
    }
    const first = items[0];
    const last = items[items.length - 1];
    const active = document.activeElement;
    if (event.shiftKey && (active === first || active === node)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  }

  node.addEventListener("keydown", onkeydown);
  return {
    update(next: FocusTrapOptions) {
      opts = next;
    },
    destroy() {
      node.removeEventListener("keydown", onkeydown);
      if (previous?.isConnected) previous.focus();
    },
  };
}
