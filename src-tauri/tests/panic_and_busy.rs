//! Long operations must always answer the WebView (issues #39 and #40).
//!
//! #40: a command that panics used to drop its response. Tauri spawns an async
//! command as a task; a panic kills the task and the IPC promise is never
//! settled, so the page that awaited it spins forever. A sync command panics on
//! the thread that serves IPC. Every command body now runs inside
//! `commands::guarded` / `commands::guarded_async`, which turn the panic into
//! an ordinary `internal` error.
//!
//! #39: execute, restore and purge had no busy guard. The database mutex
//! queued a double click instead of refusing it, so the second run started the
//! moment the first finished. They now share one quarantine operation lock and
//! the second call is refused with a readable `busy` error.

use std::sync::mpsc;
use std::time::Duration;

use serde_json::json;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{Manager, WebviewWindow, WebviewWindowBuilder};
use winknife_lib::commands::{guarded, guarded_async, Services};
use winknife_lib::core::busy::Operation;
use winknife_lib::core::error::ApiError;

/// How long a response may take before the test calls it lost. The commands
/// below answer in microseconds; a lost response never answers at all.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

/// `commands.rs`, read as text, for the coverage check.
const COMMANDS_SOURCE: &str = include_str!("../src/commands.rs");

#[tauri::command]
fn panics_sync() -> Result<(), ApiError> {
    guarded("panics_sync", || panic!("probe: sync panic"))
}

#[tauri::command]
async fn panics_async() -> Result<(), ApiError> {
    guarded_async("panics_async", async { panic!("probe: async panic") }).await
}

#[tauri::command]
async fn answers_async() -> Result<u32, ApiError> {
    guarded_async("answers_async", async { Ok(7) }).await
}

fn request(command: &str, args: serde_json::Value) -> InvokeRequest {
    InvokeRequest {
        cmd: command.to_owned(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: if cfg!(any(windows, target_os = "android")) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        }
        .parse()
        .expect("the IPC origin must parse"),
        body: InvokeBody::Json(args),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_owned(),
    }
}

/// Dispatch on a helper thread so a lost response fails the test instead of
/// hanging it.
fn dispatch(
    webview: &WebviewWindow<MockRuntime>,
    command: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, serde_json::Value> {
    let (tx, rx) = mpsc::channel();
    let webview = webview.clone();
    let request = request(command, args);
    std::thread::spawn(move || {
        let response = tauri::test::get_ipc_response(&webview, request).map(|body| {
            body.deserialize::<serde_json::Value>()
                .unwrap_or(serde_json::Value::Null)
        });
        let _ = tx.send(response);
    });
    rx.recv_timeout(RESPONSE_TIMEOUT)
        .unwrap_or_else(|_| panic!("`{command}` never answered the WebView"))
}

fn probe_app() -> (tauri::App<MockRuntime>, WebviewWindow<MockRuntime>) {
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            panics_sync,
            panics_async,
            answers_async
        ])
        .build(mock_context(noop_assets()))
        .expect("the mock app must build");
    let webview = WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::default())
        .build()
        .expect("mock webview");
    (app, webview)
}

fn code_of(error: &serde_json::Value) -> &str {
    error["code"].as_str().unwrap_or_default()
}

#[test]
fn a_panicking_async_command_answers_with_an_internal_error() {
    let (_app, webview) = probe_app();
    let error = dispatch(&webview, "panics_async", json!({})).expect_err("must be an error");
    assert_eq!(code_of(&error), "internal", "{error}");
    let message = error["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("panics_async"),
        "names the command: {error}"
    );
    assert!(
        error["detail"]
            .as_str()
            .is_some_and(|d| d.contains("probe: async panic")),
        "carries the panic message: {error}"
    );
}

#[test]
fn a_panicking_sync_command_answers_with_an_internal_error() {
    let (_app, webview) = probe_app();
    let error = dispatch(&webview, "panics_sync", json!({})).expect_err("must be an error");
    assert_eq!(code_of(&error), "internal", "{error}");
}

#[test]
fn a_guarded_command_that_does_not_panic_is_unchanged() {
    let (_app, webview) = probe_app();
    let body = dispatch(&webview, "answers_async", json!({})).expect("must succeed");
    assert_eq!(body, json!(7));
}

/// Every real command runs inside the guard, named after itself so the log
/// line points at the right one.
#[test]
fn every_command_body_is_guarded() {
    let mut lines = COMMANDS_SOURCE.lines();
    let mut unguarded = Vec::new();
    let mut seen = 0;
    while let Some(line) = lines.next() {
        if !matches!(
            line.trim(),
            "#[tauri::command]" | "#[tauri::command(async)]"
        ) {
            continue;
        }
        seen += 1;
        // The signature runs to the line that opens the body.
        let mut signature = String::new();
        for next in lines.by_ref() {
            signature.push_str(next.trim());
            signature.push(' ');
            if next.trim_end().ends_with('{') {
                break;
            }
        }
        let name = signature
            .split("fn ")
            .nth(1)
            .and_then(|rest| rest.split(['(', '<']).next())
            .unwrap_or_default()
            .to_owned();
        let first = lines.next().unwrap_or_default().trim().to_owned();
        let sync = format!("guarded(\"{name}\", ||");
        let async_ = format!("guarded_async(\"{name}\", async");
        if !first.starts_with(&sync) && !first.starts_with(&async_) {
            unguarded.push(format!("{name}: {first}"));
        }
    }
    assert!(seen > 30, "the source scan found only {seen} commands");
    assert!(
        unguarded.is_empty(),
        "these command bodies do not start inside `guarded`/`guarded_async` \
         with their own name:\n{}",
        unguarded.join("\n")
    );
}

fn real_app(sandbox: &std::path::Path) -> (tauri::App<MockRuntime>, WebviewWindow<MockRuntime>) {
    let app = mock_builder()
        .invoke_handler(winknife_lib::invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("the mock app must build");
    winknife_lib::manage_state(app.handle(), sandbox, None, true)
        .expect("the app's own startup wiring must succeed");
    let webview = WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::default())
        .build()
        .expect("mock webview");
    (app, webview)
}

/// Every command that moves things into or out of the quarantine, with
/// arguments that would otherwise get as far as the ledger.
fn quarantine_writers() -> Vec<(&'static str, serde_json::Value)> {
    let nil = uuid::Uuid::nil().to_string();
    vec![
        (
            "execute",
            json!({ "scanId": nil, "approved": [], "confirmEmptyRecycleBin": false }),
        ),
        ("quarantine_restore", json!({ "batchId": nil })),
        ("quarantine_restore_item", json!({ "itemId": nil })),
        ("quarantine_purge", json!({})),
        (
            "quarantine_purge_items",
            json!({ "itemIds": [nil], "confirmed": false }),
        ),
    ]
}

#[test]
fn a_second_quarantine_operation_is_refused_while_one_runs() {
    let sandbox = tempfile::tempdir().expect("tempdir");
    let (app, webview) = real_app(sandbox.path());

    let services = app.state::<Services>();
    let running = services
        .quarantine
        .begin(Operation::Restore)
        .expect("nothing is running yet");

    for (command, args) in quarantine_writers() {
        let error = dispatch(&webview, command, args).expect_err("must be refused");
        assert_eq!(code_of(&error), "busy", "{command}: {error}");
        let message = error["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("restore is already running"),
            "{command} says what is running: {error}"
        );
    }

    drop(running);
    // The lock is released, so the cheapest writer goes through again.
    dispatch(&webview, "quarantine_purge", json!({})).expect("purge runs once the lock is free");
}

#[test]
fn a_finished_or_failed_operation_releases_the_lock() {
    let sandbox = tempfile::tempdir().expect("tempdir");
    let (app, webview) = real_app(sandbox.path());

    // Refused on an unknown scan, after taking the lock.
    let error = dispatch(
        &webview,
        "quarantine_restore_item",
        json!({ "itemId": uuid::Uuid::nil() }),
    )
    .expect_err("an unknown item is an error");
    assert_ne!(code_of(&error), "busy", "{error}");

    let services = app.state::<Services>();
    let _guard = services
        .quarantine
        .begin(Operation::Clean)
        .expect("a failed command must not leave the lock held");
}
