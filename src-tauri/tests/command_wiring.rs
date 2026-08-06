//! **Does the app the WebView talks to actually work?**
//!
//! Nothing else in this repository asks that question. 486 unit tests exercised
//! command *bodies* and `core` logic; every one of them passed while every
//! single command in the app failed on its first invocation with
//!
//! ```text
//! state not managed for field `db` on command `start_scan`.
//! You must call `.manage()` before using this command
//! ```
//!
//! because `lib.rs` registered an `Arc<Database>` and `commands.rs` asked for a
//! `Database`. Tauri resolves managed state by `TypeId`, so those are two
//! unrelated keys — and because the lookup happens at runtime, `cargo check`,
//! `cargo clippy` and the whole unit suite were blind to it. A test of a command
//! body can never catch this: the wiring *is* the thing that broke.
//!
//! So this file stands a real Tauri app up on [`winknife_lib::manage_state`] and
//! [`winknife_lib::invoke_handler`] — the same two functions `run()` calls, not
//! a copy of them — and dispatches every registered command through the real
//! IPC path.
//!
//! # What is asserted, and what deliberately is not
//!
//! Most of these commands want a real machine and will legitimately fail here:
//! an unknown scan id, an empty quarantine, a startup entry that does not exist.
//! A failing command is fine. What is not fine is a command that never *ran*:
//!
//! 1. no response may contain `state not managed` — the wiring bug;
//! 2. no response may contain `invalid args` — which would mean the probe's
//!    arguments stopped at deserialization and the state lookup in (1) was never
//!    reached, i.e. the probe silently proves nothing.
//!
//! (2) is what keeps this file from rotting into a test that passes for the
//! wrong reason. Argument parameters are resolved in declaration order, so a
//! command like `start_scan(app, module_id: String, …, db: State<…>)` has to get
//! a well-formed `moduleId` before it can possibly reach `db`.
//!
//! # Coverage is enforced, not maintained by hand
//!
//! Three lists have to agree, and two of the tests below do nothing but check
//! that: the `#[tauri::command]` functions in `commands.rs`, the
//! `generate_handler!` list in `lib.rs`, and the probe table here. Adding a
//! command to the app without giving it a probe fails
//! [`every_registered_command_has_a_probe`]; writing one and forgetting to
//! register it fails [`every_declared_command_is_registered`].
//!
//! # Safety
//!
//! An integration test binary links the library without `cfg(test)`, so the OS
//! guards that matter here are the runtime ones: `real_os_effects_armed()` is
//! false in any process that is not `run()`, which is what stops
//! `reveal_in_explorer` opening a window and `startup_enable` writing to
//! `HKCU\…\Run` (security review H5). Everything else is pointed at a tempdir:
//! the database, the quarantine, the logs, and every path a probe passes in.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::json;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::WebviewWindowBuilder;

/// The failure this file exists to catch, verbatim from `tauri::State`'s
/// `CommandArg` impl.
const STATE_NOT_MANAGED: &str = "state not managed";

/// Tauri's argument-deserialization failure. Seeing this means the probe never
/// got as far as the state lookup, so its verdict is worthless.
const INVALID_ARGS: &str = "invalid args";

/// One command, and arguments good enough to reach its `State` parameters.
struct Probe {
    /// The name the WebView invokes.
    command: &'static str,
    /// The IPC payload. Every parameter declared *before* a `State` one has to
    /// deserialize from this, or the command stops early and proves nothing.
    args: serde_json::Value,
}

/// The probe table: one entry per registered command, checked for completeness
/// by [`every_registered_command_has_a_probe`].
///
/// Values are chosen to be refused early and cheaply — an unknown module id, a
/// nil scan id, an empty sandbox directory — so that the command resolves all of
/// its state and then declines to do real work. Nothing here scans the
/// developer's machine (CONVENTIONS.md).
fn probes(sandbox: &Path) -> Vec<Probe> {
    let root = sandbox.to_string_lossy().into_owned();
    // Nil, so every id-taking command is asked about something that cannot exist.
    let nil = uuid::Uuid::nil().to_string();

    vec![
        Probe {
            command: "list_modules",
            args: json!({}),
        },
        Probe {
            command: "dashboard_summary",
            args: json!({}),
        },
        // Refused on the command's first line, after all six arguments resolve.
        Probe {
            command: "start_scan",
            args: json!({ "moduleId": "__wiring_probe__" }),
        },
        Probe {
            command: "cancel_scan",
            args: json!({ "scanId": nil }),
        },
        // An explicit root, or the command falls back to the profile folders and
        // hashes the developer's Documents.
        Probe {
            command: "start_duplicate_scan",
            args: json!({ "roots": [&root] }),
        },
        Probe {
            command: "treemap_drives",
            args: json!({}),
        },
        Probe {
            command: "treemap_scan",
            args: json!({ "root": &root }),
        },
        Probe {
            command: "treemap_children",
            args: json!({ "root": &root, "path": &root }),
        },
        Probe {
            command: "treemap_cached",
            args: json!({ "root": &root }),
        },
        // Refused by `real_os_effects_armed()`, so no window opens.
        Probe {
            command: "reveal_in_explorer",
            args: json!({ "path": &root }),
        },
        Probe {
            command: "startup_scan",
            args: json!({}),
        },
        // An entry that does not exist: `set_enabled` returns
        // `UnknownStartupEntry` before it reaches the registry, which is itself
        // disarmed in this process.
        Probe {
            command: "startup_enable",
            args: json!({ "source": "currentUserRun", "key": "__wiring_probe__" }),
        },
        Probe {
            command: "get_recommendations",
            args: json!({ "scanId": nil }),
        },
        Probe {
            command: "scan_summary",
            args: json!({ "scanId": nil }),
        },
        // `store.get` is the first line, so the unknown scan stops it before any
        // executor gate runs. Nothing is approved and the bin is not confirmed.
        Probe {
            command: "execute",
            args: json!({
                "scanId": nil,
                "approved": [],
                "confirmEmptyRecycleBin": false,
            }),
        },
        Probe {
            command: "get_settings",
            args: json!({}),
        },
        // The `null` is a placeholder: the dispatch loop substitutes whatever
        // `get_settings` just returned, so a new field on `AppSettings` cannot
        // silently turn this probe into an `invalid args` no-op.
        Probe {
            command: "set_settings",
            args: json!({ "settingsIn": null }),
        },
        Probe {
            command: "quarantine_list",
            args: json!({}),
        },
        Probe {
            command: "quarantine_batch_items",
            args: json!({ "batchId": nil }),
        },
        Probe {
            command: "quarantine_stats",
            args: json!({}),
        },
        Probe {
            command: "quarantine_restore",
            args: json!({ "batchId": nil }),
        },
        Probe {
            command: "quarantine_restore_item",
            args: json!({ "itemId": nil }),
        },
        Probe {
            command: "quarantine_purge",
            args: json!({}),
        },
        // Armed, observed, then cancelled. Safe because `manage_state` is asked
        // for a dry run, so the timer holds a controller that only logs — and
        // no ticker is spawned here in any case.
        Probe {
            command: "shutdown_schedule",
            args: json!({ "mode": { "mode": "in", "minutes": 1 }, "action": "shutdown" }),
        },
        Probe {
            command: "shutdown_status",
            args: json!({}),
        },
        Probe {
            command: "shutdown_cancel",
            args: json!({}),
        },
    ]
}

/* -------------------------------------------------------------------------
 * The wiring test
 * ---------------------------------------------------------------------- */

/// Build the app the way `run()` does and invoke every registered command.
#[test]
fn every_registered_command_resolves_its_state() {
    let sandbox = tempfile::tempdir().expect("tempdir");

    let app = mock_builder()
        .invoke_handler(winknife_lib::invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("the mock app must build");

    // The composition root, verbatim. `None` for the resource directory makes
    // `rules_dir` fall back to the repository's `rules/`, which is what a dev
    // build does; `true` for the dry run keeps the power controller off the
    // machine.
    winknife_lib::manage_state(app.handle(), sandbox.path(), None, true)
        .expect("the app's own startup wiring must succeed");

    // The tray menu, the tick loop and the window-geometry listener reach the
    // database on an `AppHandle` rather than through IPC, so no probe below can
    // cover them. They share one accessor; this checks that it resolves. Two of
    // those four call sites used to spell the type `Database` and silently got
    // `None`, which is how the tray's "Cancel timer" came to refuse outright.
    assert!(
        winknife_lib::managed_database(app.handle()).is_some(),
        "`managed_database` must resolve against what `manage_state` registered"
    );

    let webview = WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::default())
        .build()
        .expect("mock webview");

    let mut settings_payload = json!(null);
    let mut failures: Vec<String> = Vec::new();

    for probe in probes(sandbox.path()) {
        let args = if probe.command == "set_settings" {
            json!({ "settingsIn": settings_payload })
        } else {
            probe.args.clone()
        };

        let rendered = match dispatch(&webview, probe.command, args) {
            Ok(body) => {
                if probe.command == "get_settings" {
                    settings_payload = body.clone();
                }
                format!("ok: {body}")
            }
            Err(error) => format!("err: {error}"),
        };

        if rendered.contains(STATE_NOT_MANAGED) {
            failures.push(format!("{}: {rendered}", probe.command));
        } else if rendered.contains(INVALID_ARGS) {
            failures.push(format!(
                "{}: the probe's arguments are wrong, so it never reached the state \
                 lookup and proves nothing -- {rendered}",
                probe.command
            ));
        } else {
            // Printed rather than asserted on: most of these are expected
            // refusals, and pinning them would turn this into a behaviour test.
            println!("[wiring] {:<24} {rendered}", probe.command);
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} commands never reached their own body.\n\
         Every `State<'_, T>` parameter in `commands.rs` must name the exact type \
         `manage_state` passes to `app.manage`; Tauri matches them by `TypeId`, so \
         `Arc<Database>` and `Database` are different keys.\n\n{}",
        failures.len(),
        probes(sandbox.path()).len(),
        failures.join("\n"),
    );
}

/// Send one command through the real IPC path.
fn dispatch(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    command: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, serde_json::Value> {
    let request = InvokeRequest {
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
    };

    tauri::test::get_ipc_response(webview, request).map(|body| {
        body.deserialize::<serde_json::Value>()
            .unwrap_or(serde_json::Value::Null)
    })
}

/* -------------------------------------------------------------------------
 * Keeping the three lists in step
 * ---------------------------------------------------------------------- */

/// `commands.rs`, read as text at compile time.
const COMMANDS_SOURCE: &str = include_str!("../src/commands.rs");

/// `lib.rs`, likewise — the `generate_handler!` list is a macro invocation and
/// there is no way to enumerate it at runtime.
const LIB_SOURCE: &str = include_str!("../src/lib.rs");

/// Every `#[tauri::command]` function in `commands.rs`.
fn declared_commands() -> BTreeSet<String> {
    let mut lines = COMMANDS_SOURCE.lines().peekable();
    let mut found = BTreeSet::new();
    while let Some(line) = lines.next() {
        // Exact match, so the module doc comment that quotes the attribute is
        // not mistaken for one.
        if line.trim() != "#[tauri::command]" {
            continue;
        }
        let Some(signature) = lines.peek() else {
            continue;
        };
        let name = signature
            .trim()
            .strip_prefix("pub ")
            .map(|rest| rest.strip_prefix("async ").unwrap_or(rest))
            .and_then(|rest| rest.strip_prefix("fn "))
            .map(|rest| {
                rest.split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            });
        assert!(
            name.as_deref().is_some_and(|name| !name.is_empty()),
            "a `#[tauri::command]` is not followed by a `pub fn`: {signature}"
        );
        found.insert(name.unwrap_or_default());
    }
    found
}

/// Every command in `invoke_handler`'s `generate_handler!` list.
fn registered_commands() -> BTreeSet<String> {
    let start = LIB_SOURCE
        .find("tauri::generate_handler![")
        .expect("`invoke_handler` must still use `generate_handler!`");
    let list = &LIB_SOURCE[start..];
    let end = list.find(']').expect("the handler list must be closed");

    list[..end]
        .lines()
        .filter_map(|line| line.trim().strip_prefix("commands::"))
        .map(|entry| entry.trim_end_matches(',').to_owned())
        .collect()
}

/// A command nobody registered is a button that does nothing.
#[test]
fn every_declared_command_is_registered() {
    let declared = declared_commands();
    let registered = registered_commands();

    assert!(!declared.is_empty(), "the source scan found no commands");
    assert_eq!(
        declared,
        registered,
        "\ndeclared but never registered (the UI cannot call these): {:?}\
         \nregistered but not declared here: {:?}",
        declared.difference(&registered).collect::<Vec<_>>(),
        registered.difference(&declared).collect::<Vec<_>>(),
    );
}

/// A command with no probe is a command this file does not actually test.
#[test]
fn every_registered_command_has_a_probe() {
    let registered = registered_commands();
    let probed: BTreeSet<String> = probes(Path::new("."))
        .into_iter()
        .map(|probe| probe.command.to_owned())
        .collect();

    assert_eq!(
        registered,
        probed,
        "\nregistered but not probed -- add an entry to `probes()` with arguments \
         that reach its `State` parameters: {:?}\
         \nprobed but no longer registered: {:?}",
        registered.difference(&probed).collect::<Vec<_>>(),
        probed.difference(&registered).collect::<Vec<_>>(),
    );
}
