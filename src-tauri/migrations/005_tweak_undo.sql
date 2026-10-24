-- Migration 005: the tweak engine's undo records (K-018, Phase 6).
-- One row per applied tweak, written *before* the registry write so a crash
-- in between still leaves the prior value on record. `prior_bytes` NULL means
-- the value did not exist, and undo removes it rather than inventing one.
-- A row is open until `closed_at` is set: by an undo, or by a later apply of
-- the same tweak that supersedes it.
CREATE TABLE tweak_undo (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    tweak_id      TEXT    NOT NULL,
    hive          TEXT    NOT NULL,
    subkey        TEXT    NOT NULL,
    value_name    TEXT    NOT NULL,
    prior_kind    INTEGER,
    prior_bytes   BLOB,
    applied_kind  INTEGER NOT NULL,
    applied_bytes BLOB    NOT NULL,
    applied_at    TEXT    NOT NULL,
    closed_at     TEXT,
    close_reason  TEXT CHECK (close_reason IN ('undone', 'superseded'))
);
CREATE INDEX tweak_undo_open ON tweak_undo (tweak_id) WHERE closed_at IS NULL;
