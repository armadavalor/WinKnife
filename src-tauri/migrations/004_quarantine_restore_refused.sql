-- Migration 004: remember that a restore was refused because the item is
-- EFS-encrypted and its way home crosses drives (decision b, 2026-10-12).
-- A marked item is never removed by the automatic purge (retention window or
-- size cap); only the user removes it, explicitly. Nullable, so every row an
-- earlier build wrote loads unchanged and unmarked.
ALTER TABLE quarantine ADD COLUMN restore_refused_at TEXT;
