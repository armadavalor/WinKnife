# Phase 7: accessibility and display scaling (C8)

UI only (`src/`), no backend. Checklist and result.

- [x] Keyboard: global `:focus-visible` ring; skip link lands on `<main tabindex="-1">`.
- [x] Dialogs: `focusTrap` action (`src/lib/focusTrap.ts`) focuses on open, closes on Esc, returns focus. ConfirmDialog also traps Tab; the quarantine drawer does not (it is a side panel).
- [x] Names: every input/checkbox has a label or `aria-label` (audited, no gaps); sidebar is a labelled `nav` with `aria-current`.
- [x] Live status: scan headings in cleaner, duplicates and empty-folders are `role="status"`.
- [x] Contrast (WCAG AA 4.5:1): `ink-faint` lifted in graphite/forest; mist `ink-faint`, `accent` and the four risk tokens darkened. Risk badges keep their text labels.
- [x] Reduced motion: existing `motion` store and `prefers-reduced-motion` rules reused.
- [x] Scaling: no horizontal overflow on any route at 880x560 (the minimum window, = 200% of 1760 px). Sidebar now scrolls (`overflow-y-auto`) so Settings and Quarantine stay reachable on short windows.
- [x] `svelte-check`: 0 errors, 0 warnings.

Known gap: graphite `ink-muted` on `accent-soft` is 3.88:1 (active/hover chips only).
