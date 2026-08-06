-- Migration 001 — initial schema (ARCHITECTURE section 5).
-- Timestamps are RFC 3339 UTC strings, ids are UUID strings, so the whole
-- database stays readable with any sqlite browser. This app deletes things;
-- an auditable trail matters more than a few saved bytes.

CREATE TABLE scans (
    id            TEXT    PRIMARY KEY,
    module        TEXT    NOT NULL,
    started_at    TEXT    NOT NULL,
    finished_at   TEXT,
    finding_count INTEGER NOT NULL DEFAULT 0,
    status        TEXT    NOT NULL
) STRICT;

CREATE INDEX idx_scans_module_started ON scans (module, started_at DESC);

CREATE TABLE actions (
    id                  TEXT PRIMARY KEY,
    scan_id             TEXT NOT NULL REFERENCES scans (id) ON DELETE CASCADE,
    recommendation_json TEXT NOT NULL,
    executed_at         TEXT,
    result              TEXT
) STRICT;

CREATE INDEX idx_actions_scan ON actions (scan_id);

-- One row per quarantined item. `original_path` is the only way back, so it is
-- written before the move, never after.
CREATE TABLE quarantine (
    id              TEXT    PRIMARY KEY,
    batch_id        TEXT    NOT NULL,
    original_path   TEXT    NOT NULL,
    quarantine_path TEXT    NOT NULL,
    size_bytes      INTEGER NOT NULL,
    quarantined_at  TEXT    NOT NULL,
    purge_after     TEXT    NOT NULL,
    restored_at     TEXT
) STRICT;

CREATE INDEX idx_quarantine_batch ON quarantine (batch_id);

-- Items still held: not yet restored, ordered by when they may be purged.
CREATE INDEX idx_quarantine_purge_after ON quarantine (purge_after)
    WHERE restored_at IS NULL;

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
