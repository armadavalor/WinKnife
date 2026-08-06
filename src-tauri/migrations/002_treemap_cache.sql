-- Per-drive treemap cache (module spec 04: "cache last scan per drive in
-- SQLite; staleness timestamp shown in the UI; rescan button").
--
-- Directories only, one row each. A drive with two million files has perhaps
-- fifty thousand folders, so this stays small; file-level detail is re-read
-- from the one directory the user is looking at, which is a single `read_dir`.
--
-- `drive` and `path` hold the canonical comparison key rather than the text the
-- walk produced, so two spellings of one folder cannot become two rows.

CREATE TABLE treemap_scan (
    drive           TEXT    NOT NULL PRIMARY KEY,
    root_path       TEXT    NOT NULL,
    scanned_at      TEXT    NOT NULL,
    total_bytes     INTEGER NOT NULL,
    file_count      INTEGER NOT NULL,
    dir_count       INTEGER NOT NULL,
    unreadable_dirs INTEGER NOT NULL,
    complete        INTEGER NOT NULL
) STRICT;

CREATE TABLE treemap_dir (
    drive            TEXT    NOT NULL,
    path             TEXT    NOT NULL,
    total_bytes      INTEGER NOT NULL,
    own_bytes        INTEGER NOT NULL,
    file_count       INTEGER NOT NULL,
    dir_count        INTEGER NOT NULL,
    readable         INTEGER NOT NULL,
    unreadable_below INTEGER NOT NULL,
    compressed_below INTEGER NOT NULL,
    PRIMARY KEY (drive, path)
) STRICT;

-- Autostart shortcuts this app has moved out of a Startup folder
-- (module spec 05: "move the .lnk into a module-managed disabled/ subfolder in
-- app data, recorded in the DB, restorable"). Nothing is ever deleted, so this
-- table is the only thing that knows where a shortcut came from.
CREATE TABLE startup_disabled_link (
    id            TEXT NOT NULL PRIMARY KEY,
    source        TEXT NOT NULL,
    file_name     TEXT NOT NULL,
    original_path TEXT NOT NULL,
    stored_path   TEXT NOT NULL,
    disabled_at   TEXT NOT NULL
) STRICT;

CREATE UNIQUE INDEX startup_disabled_link_origin
    ON startup_disabled_link (source, file_name);
