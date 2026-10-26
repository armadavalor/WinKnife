//! Scheduled auto-quarantine (C11, K-028) stands aside instead of moving files
//! from under the user: a cancelled scan, a clean already running, or a manual
//! scan still waiting in Review all skip the run, and a batch that fails is
//! reported as failed rather than as zero moved.
//!
//! An integration test rather than a unit test because it needs the app's real
//! state from `manage_state`, and only test binaries under `tests/` get the
//! ComCtl32 v6 manifest the Tauri runtime needs to load (see `build.rs`).

use std::sync::Arc;

use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use tauri::{App, Manager};
use uuid::Uuid;
use winknife_lib::commands::{auto_quarantine, auto_quarantine_outcome, Services};
use winknife_lib::core::busy::Operation;
use winknife_lib::core::db::Database;
use winknife_lib::core::error::ApiError;
use winknife_lib::core::state::{ScanResult, ScanStore};
use winknife_lib::modules::scheduled_scan::{AutoQuarantine, SkipReason};

fn app() -> (tempfile::TempDir, App<MockRuntime>) {
    let sandbox = tempfile::tempdir().expect("tempdir");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("mock app");
    winknife_lib::manage_state(app.handle(), sandbox.path(), None, true).expect("state");
    (sandbox, app)
}

fn empty_scan() -> ScanResult {
    ScanResult {
        scan_id: Uuid::new_v4(),
        module: "cleaner".to_owned(),
        findings: Vec::new(),
        recommendations: Vec::new(),
        skipped_rules: Vec::new(),
        cancelled: false,
    }
}

fn run(app: &App<MockRuntime>, scan: &ScanResult) -> AutoQuarantine {
    auto_quarantine(
        &app.state::<Services>(),
        &app.state::<Arc<Database>>(),
        &app.state::<ScanStore>(),
        scan,
    )
}

fn skipped(reason: SkipReason) -> AutoQuarantine {
    AutoQuarantine::Skipped { reason }
}

#[test]
fn a_cancelled_scheduled_scan_moves_nothing() {
    let (_sandbox, app) = app();
    let scan = ScanResult {
        cancelled: true,
        ..empty_scan()
    };
    assert_eq!(run(&app, &scan), skipped(SkipReason::Cancelled));
}

#[test]
fn it_skips_while_a_clean_is_running() {
    let (_sandbox, app) = app();
    let services = app.state::<Services>();
    let _clean = services
        .quarantine
        .begin(Operation::Clean)
        .expect("the user's clean holds the lock");
    assert_eq!(run(&app, &empty_scan()), skipped(SkipReason::Busy));
}

#[test]
fn it_skips_while_a_manual_scan_waits_in_review() {
    let (_sandbox, app) = app();
    app.state::<ScanStore>().insert_for_review(empty_scan());
    assert_eq!(run(&app, &empty_scan()), skipped(SkipReason::ManualReview));
}

#[test]
fn it_runs_when_nothing_is_in_the_way() {
    let (_sandbox, app) = app();
    assert!(matches!(
        run(&app, &empty_scan()),
        AutoQuarantine::Ran { moved: 0, .. }
    ));
}

#[test]
fn a_failed_batch_is_reported_as_failed() {
    let failed = auto_quarantine_outcome(Err(ApiError::new("ledger", "ledger unusable")));
    assert_eq!(failed, skipped(SkipReason::Failed));
}
