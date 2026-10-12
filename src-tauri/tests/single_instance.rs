//! A forwarded launch must not touch the shutdown timer (issue #3 regression,
//! phase 5 task 1).
//!
//! Before single-instance, a second launch built a whole second app: it opened
//! the same database and re-armed the same persisted timer, so one scheduled
//! shutdown was counting down in two processes. Now the second launch only
//! forwards its argv; the running app's `handle_second_instance` brings the
//! window back and opens a module. This pins that it does nothing else: the
//! timer re-armed once by `manage_state` is the timer that is still armed, and
//! the persisted row is untouched.
//!
//! Its own binary, because the tracing subscriber `manage_state` installs is
//! process-global.

use chrono::{Duration, Utc};
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::{Manager, WebviewWindowBuilder};

use winknife_lib::core::platform::PowerAction;
use winknife_lib::core::{db, settings};
use winknife_lib::modules::shutdown_timer::{PersistedTimer, ShutdownTimer};
use winknife_lib::PendingLaunchRoute;

fn forwarded(args: &[&str]) -> Vec<String> {
    std::iter::once("winknife.exe")
        .chain(args.iter().copied())
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_forwarded_launch_leaves_the_rearmed_timer_alone() {
    let sandbox = tempfile::tempdir().expect("tempdir");
    let now = Utc::now();
    let persisted = PersistedTimer {
        id: uuid::Uuid::new_v4(),
        action: PowerAction::Shutdown,
        target: now + Duration::hours(1),
        scheduled_at: now,
        mode: None,
    };
    {
        let conn = db::open(sandbox.path()).expect("db");
        settings::set_json(&conn, settings::KEY_ACTIVE_TIMER, &persisted).expect("persist");
    }

    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("the mock app must build");
    // Dry run: the timer holds a controller that only logs.
    let timer = winknife_lib::manage_state(app.handle(), sandbox.path(), None, true)
        .expect("startup wiring");
    let _window = WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::default())
        .build()
        .expect("mock webview");

    assert_eq!(
        timer.snapshot(),
        Some(persisted.clone()),
        "the one instance re-arms the persisted timer at start"
    );

    for _ in 0..3 {
        winknife_lib::handle_second_instance(app.handle(), forwarded(&["--module", "startup"]));
    }
    winknife_lib::handle_second_instance(app.handle(), forwarded(&[]));

    let managed = app.state::<std::sync::Arc<ShutdownTimer>>();
    assert_eq!(
        managed.snapshot(),
        Some(persisted.clone()),
        "a forwarded launch must not re-arm, replace or cancel the timer"
    );
    let stored: Option<PersistedTimer> = winknife_lib::managed_database(app.handle())
        .expect("database")
        .with(|conn| settings::get_json(conn, settings::KEY_ACTIVE_TIMER))
        .expect("read");
    assert_eq!(stored, Some(persisted), "the persisted row is untouched");

    // What it is for: the requested module is waiting for the UI, once.
    let pending = app.state::<PendingLaunchRoute>();
    assert_eq!(pending.take().as_deref(), Some("/startup"));
    assert_eq!(pending.take(), None);
}

#[test]
fn a_forwarded_unknown_module_opens_the_dashboard() {
    let sandbox = tempfile::tempdir().expect("tempdir");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("the mock app must build");
    winknife_lib::manage_state(app.handle(), sandbox.path(), None, true).expect("startup wiring");

    winknife_lib::handle_second_instance(app.handle(), forwarded(&["--module=no-such-tool"]));

    assert_eq!(
        app.state::<PendingLaunchRoute>().take().as_deref(),
        Some(winknife_lib::DASHBOARD_ROUTE)
    );
}
