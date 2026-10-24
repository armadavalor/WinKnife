<!-- durum: aktif -->
# Phase 6 — reversible tweaks (C1)

Prereq: K-018. Source: `docs/RESEARCH-FEATURES-2026-10.md` §C1. Branch `feat/tweak-engine`.

## Goal
A `tweaks` module that changes a few Windows preferences and can always put them back. The point against peers is the undo, not the size of the catalog.

## Tasks
1. **Registry surface.** `Registry::delete_value` (fake + real, real one behind `real_os_effects_armed()` like `set_value`), `REG_TYPE_DWORD`, `RegValue::dword`.
2. **Types.** `Action::ApplyTweak` (mutating) and `Justification::Tweak { tweak_id, observed }`, where `observed` is the value the user was shown (`None` = absent). Verifiable, so `validate()` lets it through.
3. **Undo table.** Migration `005_tweak_undo.sql`: `tweak_undo(id, tweak_id, hive, subkey, value_name, prior_kind, prior_bytes, applied_kind, applied_bytes, applied_at, closed_at, close_reason)`. `prior_*` NULL = the value did not exist. `close_reason` is `undone` or `superseded`.
4. **Catalog + fence** (`modules/tweaks.rs`). Each entry: id, plain-language name, reason, source link, hive, subkey, value name, target value, reversibility (`reversible` / `reversible after restart` / `not undoable`) and restart need. The fence is code, tested, and refuses any entry that is not HKCU, is `not undoable`, or whose path touches Windows Update, search, the Start menu or Defender. First catalog (all HKCU `Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced`, `REG_DWORD`):
   - show file name extensions (`HideFileExt` = 0)
   - show hidden files and folders (`Hidden` = 1)
   - open File Explorer to This PC (`LaunchTo` = 1)
5. **Apply** (executor dispatch, gate 4 = fence + "the live value is still the prior the user was shown" + "not already applied"): record the undo row, write, read back; a read-back mismatch puts the prior back and fails the item.
6. **Undo** (`tweaks_undo` command, not an executor action): refuse unless the live value still equals what was applied; restore the prior (or delete the value when the prior was absent), read back, close the row.
7. **Scan** is read-only: one row per tweak with name, reason, exact path, current value, target value, restart note, source link and the open undo id when applied. Recommendations only for tweaks not yet applied.
8. **UI** `src/routes/tweaks/+page.svelte` + sidebar entry, EN + TR keys.

## Tests (first, against `MapRegistry`)
- apply → undo → the original value is back, byte for byte
- absent → apply → undo → absent again
- the fence refuses blocked entries (non-HKCU, `not undoable`, fenced paths)
- undo refuses when the value changed after apply
- read-back mismatch rolls back and fails

## Hooks for later (do not build here)
- **C2** context-menu cleaner: write its changes through the same `tweak_undo` table (a context-menu entry is a registry value with a prior).
- **C7** drift check: compare the live value with `applied_*` of open `tweak_undo` rows; re-apply is a button and goes through Apply, so it is undoable too.
- **C10**: hooks in at the catalog (new entries go through the fence test), nothing else.

## Out of scope
HKLM writes, services, scheduled tasks, app removal, anything the fence refuses. Escalate before adding a tweak whose undo cannot be proven by the tests above.
