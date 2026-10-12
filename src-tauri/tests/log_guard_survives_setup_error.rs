//! The audit log's appender guard must outlive a setup error (issue #21
//! follow-up).
//!
//! `manage_state` used to hold the guard in a local and hand it to Tauri only
//! at the very end, so any `?` in between — a database that cannot be opened,
//! a database from a newer build, a broken rule pack — dropped it, and `run()`'s
//! own "WinKnife cannot start" line could miss the file. Its own binary, because
//! the tracing subscriber is process-global and installs once.

use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Manager;
use tracing_appender::non_blocking::WorkerGuard;

#[test]
fn the_log_guard_is_managed_even_when_the_database_cannot_open() {
    let sandbox = tempfile::tempdir().expect("tempdir");
    // A directory where the database file should be: `db::open` fails.
    std::fs::create_dir_all(sandbox.path().join("winknife.sqlite3")).expect("blocker");

    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("the mock app must build");

    let result = winknife_lib::manage_state(app.handle(), sandbox.path(), None, true);

    assert!(result.is_err(), "the blocked database must fail setup");
    assert!(
        app.try_state::<WorkerGuard>().is_some(),
        "the appender guard must already be in managed state when setup fails"
    );
}
