//! WinKnife — modular Windows utility.
//!
//! Analyze first, never delete without consent, everything reversible.
//!
//! This is the crate root and the composition root: it decides which concrete
//! implementations the rest of the app gets. In particular it is the **only**
//! place that can install the real power controller, and it only does so when
//! `--dry-run` is absent. Everything downstream holds a trait object and has no
//! way to reach the OS on its own. `main.rs` is a shim so the same entry point
//! serves the mobile target.

pub mod commands;
pub mod core;
pub mod modules;

use std::path::Path;
use std::sync::Arc;

use tauri::Manager;

use crate::core::brain::{Brain, ProtectedPaths};
use crate::core::db::{self, Database};
use crate::core::error::{CoreError, Result};
use crate::core::platform::{
    CachedProcessProbe, DryRunPower, EnvResolver, PowerController, SystemEnv, SystemLockProbe,
    SystemProcessProbe,
};
use crate::core::quarantine::{Quarantine, RetentionPolicy};
use crate::core::settings;
use crate::core::state::{CancelRegistry, ScanStore};
use crate::modules::shutdown_timer::{self, ShutdownTimer, Tick};

/// Command-line flag that makes power actions log instead of happening.
///
/// Kept permanently, not just for CI: it is how anyone can demo or test the
/// shutdown timer without losing their session (module spec 01, acceptance).
pub const DRY_RUN_FLAG: &str = "--dry-run";

/// How often the timer ticks. One second so the tray countdown is honest.
const TICK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// Whether the process was started with [`DRY_RUN_FLAG`].
pub fn is_dry_run(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter().any(|arg| arg == DRY_RUN_FLAG)
}

/// Command-line flag that opens a module's page: `--module <id>` or
/// `--module=<id>` (phase 5, task 1).
pub const MODULE_FLAG: &str = "--module";

/// The route the shell opens when a launch names no module it knows.
pub const DASHBOARD_ROUTE: &str = "/dashboard";

/// What a launch's command line asks the running app to do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchArgs {
    /// The module id after [`MODULE_FLAG`], unvalidated. `Some("")` when the
    /// flag was given without a value, which is treated like an unknown id.
    /// The last occurrence wins.
    pub module: Option<String>,
}

/// Parse a launch's arguments: the first launch's, or the ones a second launch
/// forwards to the running app. Pure; the program path in `argv[0]` is ignored
/// because it can never equal the flag.
pub fn parse_launch_args(args: impl IntoIterator<Item = String>) -> LaunchArgs {
    let mut parsed = LaunchArgs::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == MODULE_FLAG {
            parsed.module = Some(args.next().unwrap_or_default());
        } else if let Some(value) = arg
            .strip_prefix(MODULE_FLAG)
            .and_then(|rest| rest.strip_prefix('='))
        {
            parsed.module = Some(value.to_owned());
        }
    }
    parsed
}

/// The route a launch should open, or `None` to leave the window where it is.
///
/// A known module id opens `/<id>`. Anything else — a typo, a module from a
/// newer build, an empty value — is logged at warn and opens the dashboard: a
/// shortcut with a stale id should still open the app, never an error box.
pub fn launch_route(args: &LaunchArgs, is_known: impl Fn(&str) -> bool) -> Option<String> {
    let id = args.module.as_deref()?;
    if is_known(id) {
        return Some(format!("/{id}"));
    }
    tracing::warn!(
        module = id,
        "unknown module on the command line; opening the dashboard"
    );
    Some(DASHBOARD_ROUTE.to_owned())
}

/// The route a launch asked for and the UI has not opened yet.
///
/// Pull, not push: the first launch's route is set before the WebView has
/// mounted anything that could listen, so the shell asks for it once on mount
/// (`take_launch_route`). A forwarded launch sets it and then emits
/// [`LAUNCH_NAVIGATE`] as a nudge to ask again; whichever read comes first
/// takes it, so a route is opened at most once.
#[derive(Debug, Default)]
pub struct PendingLaunchRoute(std::sync::Mutex<Option<String>>);

impl PendingLaunchRoute {
    /// Replace the pending route.
    pub fn set(&self, route: Option<String>) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = route;
        }
    }

    /// Take the pending route, leaving none behind.
    pub fn take(&self) -> Option<String> {
        self.0.lock().ok().and_then(|mut slot| slot.take())
    }
}

/// Emitted to the UI when a forwarded launch left a route to open.
pub const LAUNCH_NAVIGATE: &str = "launch:navigate";

/// Record the route a launch asked for, validated against the registry.
pub fn record_launch_route<R: tauri::Runtime>(app: &tauri::AppHandle<R>, args: &LaunchArgs) {
    let Some(pending) = app.try_state::<PendingLaunchRoute>() else {
        return;
    };
    let route = match app.try_state::<core::module::ModuleRegistry>() {
        Some(registry) => launch_route(args, |id| registry.get(id).is_ok()),
        None => launch_route(args, |_| false),
    };
    if route.is_some() {
        pending.set(route);
    }
}

/// What the running app does when a second launch forwards its arguments.
///
/// It brings the main window back — shown (from the tray), un-minimised,
/// focused — and opens the requested module. Nothing else: in particular it
/// never touches the shutdown timer, which the one running instance re-armed
/// once at its own start (issue #3; `tests/single_instance.rs`).
pub fn handle_second_instance<R: tauri::Runtime>(app: &tauri::AppHandle<R>, argv: Vec<String>) {
    use tauri::Emitter;

    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    let args = parse_launch_args(argv);
    if args.module.is_some() {
        record_launch_route(app, &args);
        let _ = app.emit(LAUNCH_NAVIGATE, ());
    }
}

/// Where the rule packs live.
///
/// An installed build reads them from the bundled resource directory —
/// `tauri.conf.json` maps `../rules/` to `rules/`, and without that mapping an
/// installed app would start with no rules at all. A dev build falls back to
/// the repository copy, so editing `rules/*.json` and re-running picks the
/// change up with no copy step.
fn rules_dir(app_dir: Option<&Path>) -> std::path::PathBuf {
    if let Some(dir) = app_dir {
        let bundled = dir.join("rules");
        if bundled.is_dir() {
            return bundled;
        }
    }
    // `CARGO_MANIFEST_DIR` is `src-tauri`; the packs live one level up.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| std::path::PathBuf::from("rules"), |p| p.join("rules"))
}

/// Assemble the brain: hard gates from the real environment, packs from disk.
///
/// A rules directory that fails to load is fatal by design — a cleaner running
/// with silently missing rules is worse than one that will not start
/// (RULES-FORMAT.md).
fn build_brain(
    env: &dyn EnvResolver,
    app_data_dir: &Path,
    quarantine_root: &Path,
    rules_dir: &Path,
) -> Result<Brain> {
    let packs = core::brain::rules::load_dir(rules_dir, env)?;
    tracing::info!(
        dir = %rules_dir.display(),
        packs = packs.len(),
        "rule packs loaded"
    );
    Ok(Brain::builder()
        .protected_paths(ProtectedPaths::from_env(env, app_data_dir, quarantine_root))
        .packs(packs)
        .lock_probe(Arc::new(SystemLockProbe))
        // Cached: a browser-cache scan asks "is Chrome running?" once per
        // finding, and a `%LOCALAPPDATA%` cache is tens of thousands of files.
        .process_probe(Arc::new(CachedProcessProbe::new(Arc::new(
            SystemProcessProbe,
        ))))
        .build())
}

/// The power controller for this run.
///
/// The single call site of the real implementation in the whole crate. It only
/// does anything at all because [`run`] armed real OS effects; a test that got
/// here would get a controller that refuses (`core::platform::guard`).
fn power_controller(dry_run: bool) -> Arc<dyn PowerController> {
    if dry_run {
        tracing::warn!("dry run: power actions will be logged, not executed");
        return Arc::new(DryRunPower::new());
    }
    core::platform::real_power_controller()
}

/// Register everything the commands resolve out of Tauri's managed state.
///
/// Split out of [`run`]'s `setup` so that `tests/command_wiring.rs` can stand a
/// real app up on **exactly** this registration rather than a hand-copied
/// imitation of it. That is not tidiness: Tauri looks managed state up by
/// `TypeId`, so a `State<'_, Database>` parameter against a managed
/// `Arc<Database>` compiles, passes clippy, passes 486 unit tests, and then
/// fails every single invocation at runtime with `state not managed for field
/// `db``. The only thing that can catch that is a test that registers what
/// production registers and then dispatches through the real IPC path, and the
/// only way to keep such a test honest is for both callers to share this
/// function.
///
/// Returns the timer, which [`run`] still needs for the tray ticker.
///
/// # Errors
/// Returns an error if the module registry, the database, the rule packs or the
/// known-apps list cannot be built — all of which are fatal by design.
pub fn manage_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    app_data_dir: &Path,
    resource_dir: Option<&Path>,
    dry_run: bool,
) -> Result<Arc<ShutdownTimer>> {
    app.manage(modules::registry()?);
    app.manage(PendingLaunchRoute::default());
    app.manage(ScanStore::new());
    app.manage(CancelRegistry::new());

    // Logging first, so everything after it is on the record.
    // The appender guard goes into managed state at once, not at the end: a
    // guard held in a local was dropped by any `?` below, so the caller's own
    // "WinKnife cannot start" line (and anything else logged after a setup
    // error) could miss the file (issue #21 follow-up). Managed state lives as
    // long as the app, which also keeps buffered audit lines on exit.
    if let Some(guard) = core::logging::init(app_data_dir) {
        app.manage(guard);
    }
    let connection = db::open(app_data_dir).inspect_err(|err| {
        tracing::error!(error = %err, code = err.code(), "the database could not be opened");
    })?;

    // Validated, not trusted: the settings table is a key/value store of
    // strings, and a stored `0` used to mean "purge everything on the next
    // start" (security review H7).
    let policy = RetentionPolicy::validated(
        settings::get_i64(
            &connection,
            settings::KEY_RETENTION_DAYS,
            core::quarantine::DEFAULT_RETENTION_DAYS,
        )?,
        settings::get_u64(
            &connection,
            settings::KEY_MAX_BYTES,
            core::quarantine::DEFAULT_MAX_BYTES,
        )?,
    );
    let quarantine = Quarantine::new(app_data_dir, policy);

    // A restore killed between its claim and its move is held again (QA A9),
    // before retention so the purge sees it.
    match quarantine.recover_interrupted_restores(&connection) {
        Ok(0) => {}
        Ok(recovered) => tracing::warn!(recovered, "interrupted restores recovered"),
        Err(err) => tracing::error!(error = %err, "interrupted restore recovery failed"),
    }

    // Retention runs at start, before anything can add to the pile.
    match quarantine.purge(&connection, chrono::Utc::now()) {
        Ok(report) if !report.is_empty() => tracing::info!(
            purged = report.purged_items,
            freed_bytes = report.freed_bytes,
            "startup quarantine purge"
        ),
        Ok(_) => {}
        Err(err) => tracing::error!(error = %err, "startup quarantine purge failed"),
    }

    let rules_dir = rules_dir(resource_dir);
    let brain = build_brain(&SystemEnv, app_data_dir, quarantine.root(), &rules_dir)?;

    let timer = Arc::new(ShutdownTimer::new(power_controller(dry_run)));
    // Re-arm a timer that survived a restart, if its target is still ahead;
    // discard it otherwise.
    if let Some(persisted) = settings::get_json(&connection, settings::KEY_ACTIVE_TIMER)? {
        if !timer.rearm(persisted, chrono::Utc::now()) {
            settings::clear(&connection, settings::KEY_ACTIVE_TIMER)?;
        }
    }

    // The startup manager is composed here, like everything else that can touch
    // the machine: it holds the real registry, the real task scheduler and the
    // real signature probe, and every one of those refuses to write until the
    // flag armed in `run()` is set.
    let database = Arc::new(Database::new(connection));
    let startup_service = Arc::new(modules::startup::StartupService::new(
        Arc::new(core::platform::SystemRegistry),
        Arc::new(core::platform::SystemTaskScheduler),
        Arc::new(core::platform::SystemSignatureProbe),
        modules::startup::KnownApps::load(&rules_dir)?,
        modules::startup::StartupPaths::from_env(&SystemEnv, app_data_dir),
        SystemEnv.var("WINDIR").map(std::path::PathBuf::from),
        quarantine.clone(),
    ));

    // Restore points: the real platform refuses until `run()` has armed it, and
    // the registry is only read (module spec 09).
    app.manage(Arc::new(modules::restore_point::RestorePointService::new(
        Arc::new(core::platform::SystemRestorePoints),
        Arc::new(core::platform::SystemRegistry),
    )));

    // Hosts editor (K-019): the fence is resolved from %SystemRoot% once, and
    // the elevated helper refuses until `run()` has armed real OS effects.
    app.manage(Arc::new(modules::hosts::HostsService::new(
        core::platform::HostsFence::system(&SystemEnv)?,
        Arc::new(core::platform::ElevatedHostsWriter),
        app_data_dir.to_path_buf(),
    )));

    // Legacy panels: only opens Windows' own windows, gated like Explorer
    // (module spec 10).
    app.manage(Arc::new(modules::legacy_panels::LegacyPanelsService::new(
        Arc::new(core::platform::SystemPanelLauncher),
        SystemEnv.var("SystemRoot").map(std::path::PathBuf::from),
    )));

    app.manage(commands::Services {
        quarantine,
        brain,
        recycle_bin: core::platform::real_recycle_bin(),
        startup: Arc::new(modules::startup::StartupControlAdapter::new(Arc::clone(
            &startup_service,
        ))),
        startup_service,
        // Like the startup manager, it holds the real registry, which refuses
        // to write until `run()` has armed it.
        tweaks: Arc::new(modules::tweaks::TweakService::new(Arc::new(
            core::platform::SystemRegistry,
        ))),
        // Reads both hives; writes only under HKCU, and only once armed.
        context_menu: Arc::new(modules::context_menu::ContextMenuService::new(
            Arc::new(core::platform::SystemRegistry),
            Arc::new(core::platform::SystemSignatureProbe),
        )),
        rules_dir,
    });
    // `Arc<Database>`, as every
    // `db:` parameter in `commands.rs` and every `try_state` in this file has to
    // spell it the same way.
    app.manage(database);
    app.manage(Arc::clone(&timer));
    app.manage(Arc::new(modules::keep_awake::build(app, dry_run)));

    Ok(timer)
}

/// Every command the WebView is allowed to call.
///
/// Split out of [`run`] for the same reason as [`manage_state`]: the wiring test
/// registers this exact list, so a command added to the app is a command the
/// test dispatches.
pub fn invoke_handler<R: tauri::Runtime>(
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::list_modules,
        commands::dashboard_summary,
        commands::start_scan,
        commands::cancel_scan,
        commands::start_duplicate_scan,
        commands::set_folder_keep_rule,
        commands::ignore_folder_group,
        commands::clear_ignored_folder_groups,
        commands::start_empty_folder_scan,
        commands::treemap_drives,
        commands::treemap_scan,
        commands::treemap_children,
        commands::treemap_cached,
        commands::reveal_in_explorer,
        commands::startup_scan,
        commands::startup_enable,
        commands::tweaks_scan,
        commands::tweaks_undo,
        commands::context_menu_scan,
        commands::context_menu_undo,
        commands::get_recommendations,
        commands::scan_summary,
        commands::execute,
        commands::get_settings,
        commands::set_settings,
        commands::quarantine_list,
        commands::quarantine_batch_items,
        commands::quarantine_stats,
        commands::quarantine_restore,
        commands::quarantine_restore_item,
        commands::quarantine_purge,
        commands::quarantine_purge_item,
        commands::shutdown_schedule,
        commands::shutdown_cancel,
        commands::shutdown_status,
        commands::take_launch_route,
        commands::restore_point_status,
        commands::restore_point_create,
        commands::restore_point_open_settings,
        commands::hosts_read,
        commands::hosts_preview,
        commands::hosts_save,
        commands::hosts_backups,
        commands::hosts_restore,
        commands::hosts_reveal_backup,
        commands::hosts_flush_dns,
        commands::legacy_panels_list,
        commands::legacy_panels_open,
        commands::keep_awake_start,
        commands::keep_awake_stop,
        commands::keep_awake_status,
        commands::unlocker_query,
        commands::unlocker_request_close,
    ]
}

/// The database, for the handful of places that reach it outside a command.
///
/// The tray menu, the tick loop and the window-geometry listener all run on an
/// `AppHandle` rather than through IPC, so no command signature covers them and
/// `tests/command_wiring.rs` cannot dispatch to them. Each used to name the type
/// itself, and two of the four named it wrong: `try_state::<Database>()` against
/// a managed `Arc<Database>` returns `None`, which made the tray's "Cancel
/// timer" refuse outright and left a fired timer sitting in the settings table
/// to re-arm on the next start. One accessor means one spelling to get right,
/// next to the `app.manage` call that decides it.
///
/// Generic over the runtime so `tests/command_wiring.rs` can check it against
/// the mock app it already builds.
pub fn managed_database<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Option<tauri::State<'_, Arc<Database>>> {
    app.try_state::<Arc<Database>>()
}

/// Build and run the desktop application.
///
/// # Panics
/// Panics if Tauri itself cannot start (no WebView2 runtime, broken bundle
/// context). There is no UI to report the failure in at that point, so failing
/// loudly is the only honest option.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // The composition root, and the only place allowed to do this: from here on
    // the real power controller and the real recycle bin will act rather than
    // refuse. Nothing else in the crate — and nothing at all outside it — can
    // reach this, which is what keeps every test binary harmless
    // (`core::platform::guard`, security review H5).
    //
    // The `cfg` mirrors the definition: the arming function does not exist in a
    // test build at all, so a `#[cfg(test)]` module cannot call it — not even
    // by going through `run()` (security review M12). `run()` is the
    // application entry point and is never called from a test.
    #[cfg(not(test))]
    core::platform::arm_real_os_effects();

    // The elevated restore-point helper (module spec 09) stops here: no log
    // file, no database, no window and — above all — no single-instance
    // plugin, which would forward this argv to the running app and exit with
    // no restore point made. Its exit code is the whole answer.
    // The elevated hosts writer (K-019) stops here for the same reasons.
    if let Some(code) = core::platform::run_helper_if_asked(std::env::args()) {
        std::process::exit(i32::try_from(code).unwrap_or(i32::MAX));
    }
    let launch = match modules::restore_point::launch_mode(std::env::args()) {
        modules::restore_point::LaunchMode::CreateRestorePoint { description } => {
            let code = core::platform::create_in_helper(&description);
            std::process::exit(i32::try_from(code).unwrap_or(i32::MAX));
        }
        modules::restore_point::LaunchMode::App(launch) => launch,
    };
    let dry_run = is_dry_run(std::env::args());

    tauri::Builder::default()
        // First, so a second launch is turned away before any other plugin or
        // `setup` runs in it: it forwards its argv here and exits, and never
        // opens the database or re-arms a persisted timer (issue #3).
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            handle_second_instance(app, argv);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .map_err(|err| CoreError::AppDataDir(format!("{err}")))?;
            let resource_dir = app.path().resource_dir().ok();

            // Without a console a setup error used to end the process with
            // nothing on screen; say why first (round 3: a database from a
            // newer build is refused rather than opened).
            let timer = manage_state(
                app.handle(),
                &app_data_dir,
                resource_dir.as_deref(),
                dry_run,
            )
            .inspect_err(|err| {
                tracing::error!(error = %err, code = err.code(), "WinKnife cannot start");
                core::platform::show_fatal_error("WinKnife cannot start", &err.to_string());
            })?;

            record_launch_route(app.handle(), &launch);
            tray::install(app.handle())?;
            spawn_ticker(app.handle().clone(), timer);
            window::restore_and_track(app.handle());

            Ok(())
        })
        .invoke_handler(invoke_handler())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Drive the timer once a second: emit a tick, fire warnings, refresh the tray.
fn spawn_ticker(app: tauri::AppHandle, timer: Arc<ShutdownTimer>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio_sleep(TICK_INTERVAL).await;
            match timer.tick(chrono::Utc::now()) {
                Tick::Idle => tray::set_tooltip(&app, None),
                Tick::Counting {
                    remaining_secs,
                    warning,
                } => {
                    tray::set_tooltip(&app, Some(remaining_secs));
                    let _ = events::emit_shutdown_tick(&app, remaining_secs);
                    if let Some(minutes) = warning {
                        notify_warning(&app, minutes, remaining_secs);
                    }
                }
                Tick::Fired { action } | Tick::Failed { action, .. } => {
                    tray::set_tooltip(&app, None);
                    let _ = events::emit_shutdown_tick(&app, 0);
                    tracing::info!(action = action.slug(), "timer resolved");
                    clear_persisted_timer(&app);
                }
            }
        }
    });
}

async fn tokio_sleep(duration: std::time::Duration) {
    // Tauri's async runtime is tokio; going through its re-export keeps the
    // dependency list honest.
    tauri::async_runtime::spawn_blocking(move || std::thread::sleep(duration))
        .await
        .ok();
}

fn clear_persisted_timer(app: &tauri::AppHandle) {
    if let Some(db) = managed_database(app) {
        let _ = db.with(|conn| settings::clear(conn, settings::KEY_ACTIVE_TIMER));
    }
}

fn notify_warning(app: &tauri::AppHandle, minutes: i64, remaining_secs: i64) {
    use tauri_plugin_notification::NotificationExt;

    let body = format!(
        "The computer is scheduled in {}. Open WinKnife to cancel.",
        shutdown_timer::format_remaining(remaining_secs)
    );
    if let Err(err) = app
        .notification()
        .builder()
        .title(format!("Shutdown in {minutes} minute(s)"))
        .body(body)
        .show()
    {
        tracing::debug!(error = %err, "warning notification was not shown");
    }
}

/// Window geometry: put it back where the user left it, and remember it when
/// they move it.
mod window {
    use tauri::{Listener, Manager, PhysicalPosition, PhysicalSize};

    use crate::core::window_state::{self, WindowState};

    /// Restore the saved geometry and start recording changes.
    ///
    /// Every step is best-effort: a utility that refused to open because it
    /// could not remember where it was last time would be a poor trade.
    pub fn restore_and_track(app: &tauri::AppHandle) {
        let Some(window) = app.get_webview_window("main") else {
            return;
        };

        if let Some(db) = crate::managed_database(app) {
            match db.with(window_state::load) {
                Ok(Some(state)) => {
                    let _ = window.set_size(PhysicalSize::new(state.width, state.height));
                    let _ = window.set_position(PhysicalPosition::new(state.x, state.y));
                    if state.maximized {
                        let _ = window.maximize();
                    }
                }
                Ok(None) => {}
                Err(err) => tracing::debug!(error = %err, "window geometry was not restored"),
            }
        }

        // Saved when the window closes rather than on every move: a drag emits
        // hundreds of events and a settings write per pixel is absurd.
        let handle = app.clone();
        window.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                save(&handle);
            }
        });
        // Also on the app-wide exit path, because a tray "Quit" never asks the
        // window to close.
        let handle = app.clone();
        app.listen("tauri://close-requested", move |_| save(&handle));
    }

    fn save(app: &tauri::AppHandle) {
        let Some(window) = app.get_webview_window("main") else {
            return;
        };
        let Some(db) = crate::managed_database(app) else {
            return;
        };
        let (Ok(size), Ok(position)) = (window.outer_size(), window.outer_position()) else {
            return;
        };
        let state = WindowState {
            width: size.width,
            height: size.height,
            x: position.x,
            y: position.y,
            maximized: window.is_maximized().unwrap_or(false),
        };
        if let Err(err) = db.with(|conn| window_state::save(conn, &state)) {
            tracing::debug!(error = %err, "window geometry was not saved");
        }
    }
}

/// The `shutdown:tick` event, kept here because it is the only event with a
/// module-specific name (ARCHITECTURE section 3 lists the shared four).
mod events {
    use tauri::{AppHandle, Emitter};

    /// Emitted once a second while a timer is armed.
    pub const SHUTDOWN_TICK: &str = "shutdown:tick";

    pub fn emit_shutdown_tick(app: &AppHandle, remaining_secs: i64) -> tauri::Result<()> {
        app.emit(
            SHUTDOWN_TICK,
            serde_json::json!({ "remainingSecs": remaining_secs }),
        )
    }
}

/// Tray icon: countdown tooltip plus a cancel item, so the timer can be called
/// off without opening the window (module spec 01).
mod tray {
    use std::sync::Arc;

    use tauri::menu::{Menu, MenuEvent, MenuItem};
    use tauri::tray::{TrayIcon, TrayIconBuilder};
    use tauri::{AppHandle, Manager};

    use crate::core::settings;
    use crate::modules::shutdown_timer::{format_remaining, ShutdownTimer};

    const TRAY_ID: &str = "winknife-tray";
    const ITEM_SHOW: &str = "show";
    const ITEM_CANCEL: &str = "cancel-timer";
    const ITEM_QUIT: &str = "quit";

    /// Build the tray and its menu.
    pub fn install(app: &AppHandle) -> tauri::Result<()> {
        let show = MenuItem::with_id(app, ITEM_SHOW, "Open WinKnife", true, None::<&str>)?;
        let cancel = MenuItem::with_id(app, ITEM_CANCEL, "Cancel timer", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, ITEM_QUIT, "Quit", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&show, &cancel, &quit])?;

        TrayIconBuilder::with_id(TRAY_ID)
            .icon(
                app.default_window_icon()
                    .cloned()
                    .ok_or_else(|| tauri::Error::UnknownPath)?,
            )
            .tooltip("WinKnife")
            .menu(&menu)
            .show_menu_on_left_click(true)
            .on_menu_event(handle_menu_event)
            .build(app)?;
        Ok(())
    }

    fn handle_menu_event(app: &AppHandle, event: MenuEvent) {
        match event.id().as_ref() {
            ITEM_SHOW => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            ITEM_CANCEL => cancel_timer(app),
            ITEM_QUIT => app.exit(0),
            other => tracing::debug!(item = other, "unhandled tray menu item"),
        }
    }

    /// How many times the tray retries clearing the persisted timer before it
    /// gives up and leaves the countdown running.
    const CANCEL_ATTEMPTS: u32 = 3;

    /// Cancel from the tray, in the same order the command uses: persistence
    /// first, memory second.
    ///
    /// This used to be `let _ = db.with(...)` *after* `timer.cancel()`, so a
    /// failed write silently resurrected the shutdown on the next app start —
    /// and the user, having watched the countdown disappear, had no way to know
    /// (security review H3). The write is retried, and if it will not go
    /// through the timer stays armed and the tooltip keeps counting, because a
    /// visible timer the user can cancel again beats an invisible one that
    /// comes back.
    fn cancel_timer(app: &AppHandle) {
        let Some(db) = crate::managed_database(app) else {
            tracing::error!("no database is registered; the timer cannot be cancelled safely");
            return;
        };

        let mut last = None;
        for attempt in 1..=CANCEL_ATTEMPTS {
            match db.with(|conn| settings::clear(conn, settings::KEY_ACTIVE_TIMER)) {
                Ok(()) => {
                    if let Some(timer) = app.try_state::<Arc<ShutdownTimer>>() {
                        timer.cancel();
                    }
                    set_tooltip(app, None);
                    return;
                }
                Err(err) => {
                    tracing::warn!(attempt, error = %err, "clearing the persisted timer failed");
                    last = Some(err);
                }
            }
        }

        tracing::error!(
            error = ?last.map(|err| err.to_string()),
            "could not clear the persisted timer after {CANCEL_ATTEMPTS} attempts; \
             leaving it armed rather than letting it come back on the next start"
        );
    }

    /// Refresh the tooltip. `None` means no timer is armed.
    pub fn set_tooltip(app: &AppHandle, remaining_secs: Option<i64>) {
        let Some(tray) = app.tray_by_id(TRAY_ID) else {
            return;
        };
        let text = match remaining_secs {
            Some(secs) => format!("WinKnife — {} left", format_remaining(secs)),
            None => "WinKnife".to_owned(),
        };
        let _: Result<(), _> = TrayIcon::set_tooltip(&tray, Some(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dry_run_flag_is_detected() {
        assert!(is_dry_run([
            "winknife.exe".to_owned(),
            DRY_RUN_FLAG.to_owned()
        ]));
        assert!(!is_dry_run(["winknife.exe".to_owned()]));
        assert!(!is_dry_run(["winknife.exe".to_owned(), "--dry".to_owned()]));
    }

    fn argv(args: &[&str]) -> Vec<String> {
        std::iter::once("winknife.exe")
            .chain(args.iter().copied())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn the_module_flag_is_parsed_in_both_spellings() {
        let expected = Some("startup".to_owned());
        assert_eq!(
            parse_launch_args(argv(&["--module", "startup"])).module,
            expected
        );
        assert_eq!(
            parse_launch_args(argv(&["--module=startup"])).module,
            expected
        );
        assert_eq!(
            parse_launch_args(argv(&["--dry-run", "--module", "startup"])).module,
            expected
        );
    }

    #[test]
    fn no_module_flag_means_no_module() {
        assert_eq!(parse_launch_args(argv(&[])), LaunchArgs::default());
        assert_eq!(parse_launch_args(argv(&["--dry-run"])).module, None);
        // Only the exact flag counts.
        assert_eq!(parse_launch_args(argv(&["--modules=startup"])).module, None);
        assert_eq!(parse_launch_args(argv(&["startup"])).module, None);
    }

    #[test]
    fn a_module_flag_without_a_value_is_an_empty_id() {
        assert_eq!(
            parse_launch_args(argv(&["--module"])).module,
            Some(String::new())
        );
        assert_eq!(
            parse_launch_args(argv(&["--module="])).module,
            Some(String::new())
        );
    }

    #[test]
    fn the_last_module_flag_wins() {
        assert_eq!(
            parse_launch_args(argv(&["--module", "cleaner", "--module=startup"])).module,
            Some("startup".to_owned())
        );
    }

    #[test]
    fn a_known_module_opens_its_route_and_anything_else_the_dashboard() {
        let known = |id: &str| id == "startup";
        let route = |args: &[&str]| launch_route(&parse_launch_args(argv(args)), known);

        assert_eq!(route(&[]), None, "no flag leaves the window where it is");
        assert_eq!(route(&["--module", "startup"]).as_deref(), Some("/startup"));
        assert_eq!(
            route(&["--module", "nope"]).as_deref(),
            Some(DASHBOARD_ROUTE)
        );
        assert_eq!(route(&["--module"]).as_deref(), Some(DASHBOARD_ROUTE));
        // An id is a registry key, not a path: nothing else gets through.
        assert_eq!(
            route(&["--module=../settings"]).as_deref(),
            Some(DASHBOARD_ROUTE)
        );
    }

    #[test]
    fn a_pending_route_is_taken_once() {
        let pending = PendingLaunchRoute::default();
        assert_eq!(pending.take(), None);
        pending.set(Some("/startup".to_owned()));
        assert_eq!(pending.take().as_deref(), Some("/startup"));
        assert_eq!(pending.take(), None, "a route is opened at most once");
    }

    #[test]
    fn dry_run_selects_the_logging_power_controller() {
        assert!(power_controller(true).is_dry_run());
    }

    #[test]
    fn the_rules_directory_falls_back_to_the_repository_copy() {
        let dir = rules_dir(None);
        assert!(
            dir.join("windows-temp.json").is_file(),
            "expected the shipped packs at {}",
            dir.display()
        );
    }

    #[test]
    fn a_bundled_rules_directory_wins_over_the_repository_copy() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(temp.path().join("rules")).expect("rules dir");
        assert_eq!(rules_dir(Some(temp.path())), temp.path().join("rules"));
    }

    #[test]
    fn the_brain_built_from_the_real_layout_has_gates_and_packs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app_data = temp.path().join("appdata");
        let quarantine = app_data.join("quarantine");
        // The profile has to be the tempdir the packs expand into: a rule that
        // expands outside the declared profile now has to say `requires_admin`
        // (security review M8), and the shipped packs do not.
        let env = core::platform::MapEnv::new()
            .with("WINDIR", "C:\\Windows")
            .with("USERPROFILE", temp.path().to_string_lossy())
            .with("TEMP", temp.path().join("temp").to_string_lossy())
            .with("LOCALAPPDATA", temp.path().join("local").to_string_lossy());

        let brain = build_brain(&env, &app_data, &quarantine, &rules_dir(None))
            .expect("the shipped layout must produce a working brain");

        assert!(!brain.packs().is_empty(), "packs must load");
        assert!(
            brain
                .protection_reason(Path::new("C:\\Windows\\System32\\a.dll"))
                .is_some(),
            "hard gates must be armed"
        );
    }

    /// **Can the app start on the machine running this test?**
    ///
    /// Every other test in this crate hands the brain a [`MapEnv`] full of
    /// fabricated values, so until this one existed nothing had ever loaded the
    /// shipped packs against the environment the app actually boots in — and
    /// that is precisely where it did not boot. Windows hands `%TEMP%` over in
    /// 8.3 form on a great many accounts (`C:\Users\USERNA~1\AppData\Local\Temp`,
    /// `C:\Users\RUNNER~1\…` on the GitHub runner) while `%USERPROFILE%` arrives
    /// in full. Compared as text those are two different places, the profile
    /// containment check said the temp rules reach outside the profile, and
    /// `build_brain` failing is fatal by design: `run()`'s `setup` propagates it
    /// and no window ever opens.
    ///
    /// [`MapEnv`]: core::platform::MapEnv
    #[test]
    fn the_shipped_packs_load_against_the_real_process_environment() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app_data = temp.path().join("appdata");
        let quarantine = app_data.join("quarantine");

        let brain = build_brain(&SystemEnv, &app_data, &quarantine, &rules_dir(None))
            .unwrap_or_else(|err| {
                let var = |name: &str| SystemEnv.var(name).unwrap_or_else(|| "<unset>".to_owned());
                panic!(
                    "WinKnife cannot start on this machine.\n\
                     *** LOAD FAILED *** {err}\n\
                     TEMP         = {}\n\
                     LOCALAPPDATA = {}\n\
                     USERPROFILE  = {}",
                    var("TEMP"),
                    var("LOCALAPPDATA"),
                    var("USERPROFILE"),
                )
            });

        assert!(!brain.packs().is_empty(), "the shipped packs must load");
        assert!(
            brain
                .protection_reason(Path::new("C:\\Windows\\System32\\a.dll"))
                .is_some(),
            "hard gates must be armed"
        );
    }

    /// The exact shape that kept the window from opening, pinned so it cannot
    /// come back: a short-form `%TEMP%` nested inside a long-form
    /// `%USERPROFILE%`. Two spellings of one directory, and the containment
    /// check has to see through both.
    ///
    /// Built out of real directories rather than invented strings, because
    /// seeing through a spelling is exactly the part that needs a disk.
    #[cfg(windows)]
    #[test]
    fn a_short_form_temp_inside_a_long_form_profile_still_loads() {
        let dir = tempfile::tempdir().expect("tempdir");
        // The long spelling, whatever the tempdir handed us.
        let profile = dunce::canonicalize(dir.path()).expect("canonicalize");
        let temp = profile.join("AppData").join("Local").join("Temp");
        let local_app_data = profile.join("AppData").join("Local");
        std::fs::create_dir_all(temp.join("Microsoft").join("Windows").join("Explorer"))
            .expect("fixture tree");

        // …and the short one, which is what Windows really puts in `%TEMP%` on
        // an account whose name does not fit 8.3.
        let short_temp = core::test_support::short_path_name(&temp);
        let Some(short_temp) = short_temp.filter(|short| short != &temp) else {
            core::test_support::privileged_operation_unavailable(
                "a_short_form_temp_inside_a_long_form_profile_still_loads",
                "no 8.3 alias is available here (fsutil 8dot3name), so the two spellings \
                 this test compares cannot be produced",
            );
            return;
        };

        let env = core::platform::MapEnv::new()
            .with("WINDIR", "C:\\Windows")
            .with("USERPROFILE", profile.to_string_lossy())
            .with("TEMP", short_temp.to_string_lossy())
            .with("LOCALAPPDATA", local_app_data.to_string_lossy());

        let app_data = profile.join("appdata");
        build_brain(
            &env,
            &app_data,
            &app_data.join("quarantine"),
            &rules_dir(None),
        )
        .unwrap_or_else(|err| {
            panic!(
                "a short-form `%TEMP%` is still inside a long-form `%USERPROFILE%`.\n\
                     *** LOAD FAILED *** {err}\n\
                     TEMP        = {}\n\
                     USERPROFILE = {}",
                short_temp.display(),
                profile.display(),
            )
        });
    }

    #[test]
    fn a_broken_rules_directory_refuses_to_build_a_brain() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("broken.json"), "{ nope").expect("write");
        let err = build_brain(
            &core::platform::MapEnv::new(),
            temp.path(),
            &temp.path().join("q"),
            temp.path(),
        )
        .expect_err("a broken pack must not start silently");
        assert!(matches!(err, CoreError::RulePack { .. }));
    }
}
