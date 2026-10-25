<!-- durum: aktif -->
# Phase 7 — environment variables (C15)

Prereq: K-024, K-018 (C1 tweak engine). Source: `docs/RESEARCH-FEATURES-2026-10.md` §C15, issue #19. Branch `claude/c15-env-vars-3f5ee9`.

## Goal
An `env-vars` page that lists user and system environment variables and edits the user ones. The point against peers is the snapshot and the diff preview: every change shows old → new first and can be undone.

## Tasks
1. **Module** `modules/env_vars.rs`, id `env-vars`, registered in the module registry, sidebar entry, route `src/routes/env-vars/+page.svelte`.
2. **Listing** (read-only):
   - user variables from `HKCU\Environment`;
   - system variables from `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`, labelled read-only;
   - a user value that is not `REG_SZ` / `REG_EXPAND_SZ` is shown read-only with a reason.
3. **PATH-like values** (any value containing `;`): split into entries; each entry can be flagged with a reason:
   - empty entry;
   - duplicate (case-insensitive, trailing `\` ignored);
   - missing directory (only for entries that look like a path: contain `\`, `/`, `:` or `%`; checked after `%VAR%` expansion; an unknown `%VAR%` is its own flag).
   The UI edits them as rows: reorder, add, remove.
4. **Change** = `{ name, scope, new: value + expand | none (delete), expected: what the user was shown }`. Refused before any registry call when:
   - scope is `system` (HKLM is never written);
   - the name is empty, contains `=`, or is longer than 16 383 characters;
   - the value is longer than 32 766 characters;
   - the live value is no longer what the user was shown (or, when adding, the name already exists).
5. **Preview** returns before, after and a diff (per entry for list values, per line otherwise). The UI shows it in a confirm dialog; nothing is written before confirm.
6. **Apply** goes through the C1 `tweak_undo` table, tweak id `env:<NAME>` (upper case):
   - record the prior value (or "absent") before the write;
   - write with the exact kind the user picked (`REG_SZ` vs `REG_EXPAND_SZ`); a modify keeps the existing value name's casing;
   - read back; a mismatch puts the prior back and fails;
   - a delete is a C1 row whose applied value is the sentinel `REG_NONE` + empty bytes ("absent after");
   - broadcast `WM_SETTINGCHANGE` "Environment" with `SendMessageTimeoutW(SMTO_ABORTIFHUNG, 1000 ms)`, only when `real_os_effects_armed()`.
7. **Undo** (`env_vars_undo`): one step back, like C1. Refuses unless the live value still equals what WinKnife applied (for a delete: still absent); restores the prior (or deletes the value when the prior was absent), reads back, closes the row, broadcasts.

## Tests (first, against `MapRegistry` + in-memory DB)
- PATH split / join round-trip
- duplicate, missing, empty and unknown-variable flags
- name and value validation
- create → undo → absent; modify → undo → old value byte for byte, kind kept; delete → undo → old value back with its kind
- `REG_EXPAND_SZ` stays `REG_EXPAND_SZ` through modify and undo
- any `system` change is refused and the HKLM key is untouched
- stale `expected` is refused; undo refuses after an outside change

## Design notes
- Env changes do not go through the executor or the recommendation list (this page has no scan/execute), so `TweakRouter` is not touched; the page calls its own commands, like the hosts editor.
- `env:` rows live in `tweak_undo` next to `context-menu:` and C1 rows; nothing else reads them.

## Out of scope
HKLM writes (would need admin, K-021), per-process environment, volatile env (`HKCU\Volatile Environment`), import/export, multi-step history.
