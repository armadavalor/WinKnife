//! Cleanify — modular Windows utility.
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
    DryRunPower, EnvResolver, PowerController, SystemEnv, SystemLockProbe,
};
use crate::core::quarantine::{Quarantine, RetentionPolicy};
use crate::core::settings;
use crate::core::state::ScanStore;
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
    core::platform::arm_real_os_effects();

    let dry_run = is_dry_run(std::env::args());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
            app.manage(modules::registry()?);
            app.manage(ScanStore::new());

            let app_data_dir = app
                .path()
                .app_data_dir()
                .map_err(|err| CoreError::AppDataDir(format!("{err}")))?;
            // Logging first, so everything after it is on the record.
            let log_guard = core::logging::init(&app_data_dir);
            let connection = db::open(&app_data_dir)?;

            // Validated, not trusted: the settings table is a key/value store
            // of strings, and a stored `0` used to mean "purge everything on
            // the next start" (security review H7).
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
            let quarantine = Quarantine::new(&app_data_dir, policy);

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

            let rules_dir = rules_dir(app.path().resource_dir().ok().as_deref());
            let brain = build_brain(&SystemEnv, &app_data_dir, quarantine.root(), &rules_dir)?;

            let timer = Arc::new(ShutdownTimer::new(power_controller(dry_run)));
            // Re-arm a timer that survived a restart, if its target is still
            // ahead; discard it otherwise.
            if let Some(persisted) = settings::get_json(&connection, settings::KEY_ACTIVE_TIMER)? {
                if !timer.rearm(persisted, chrono::Utc::now()) {
                    settings::clear(&connection, settings::KEY_ACTIVE_TIMER)?;
                }
            }

            app.manage(commands::Services {
                quarantine,
                brain,
                recycle_bin: core::platform::real_recycle_bin(),
                rules_dir,
            });
            app.manage(Database::new(connection));
            app.manage(Arc::clone(&timer));
            if let Some(guard) = log_guard {
                // The appender guard has to outlive the process, or buffered
                // audit lines are lost on exit.
                app.manage(guard);
            }

            tray::install(app.handle())?;
            spawn_ticker(app.handle().clone(), timer);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_modules,
            commands::start_scan,
            commands::get_recommendations,
            commands::scan_summary,
            commands::execute,
            commands::quarantine_list,
            commands::quarantine_batch_items,
            commands::quarantine_stats,
            commands::quarantine_restore,
            commands::quarantine_restore_item,
            commands::quarantine_purge,
            commands::shutdown_schedule,
            commands::shutdown_cancel,
            commands::shutdown_status,
        ])
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
    if let Some(db) = app.try_state::<Database>() {
        let _ = db.with(|conn| settings::clear(conn, settings::KEY_ACTIVE_TIMER));
    }
}

fn notify_warning(app: &tauri::AppHandle, minutes: i64, remaining_secs: i64) {
    use tauri_plugin_notification::NotificationExt;

    let body = format!(
        "The computer is scheduled in {}. Open Cleanify to cancel.",
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

    use crate::core::db::Database;
    use crate::core::settings;
    use crate::modules::shutdown_timer::{format_remaining, ShutdownTimer};

    const TRAY_ID: &str = "cleanify-tray";
    const ITEM_SHOW: &str = "show";
    const ITEM_CANCEL: &str = "cancel-timer";
    const ITEM_QUIT: &str = "quit";

    /// Build the tray and its menu.
    pub fn install(app: &AppHandle) -> tauri::Result<()> {
        let show = MenuItem::with_id(app, ITEM_SHOW, "Open Cleanify", true, None::<&str>)?;
        let cancel = MenuItem::with_id(app, ITEM_CANCEL, "Cancel timer", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, ITEM_QUIT, "Quit", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&show, &cancel, &quit])?;

        TrayIconBuilder::with_id(TRAY_ID)
            .icon(
                app.default_window_icon()
                    .cloned()
                    .ok_or_else(|| tauri::Error::UnknownPath)?,
            )
            .tooltip("Cleanify")
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
        let Some(db) = app.try_state::<Database>() else {
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
            Some(secs) => format!("Cleanify — {} left", format_remaining(secs)),
            None => "Cleanify".to_owned(),
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
            "cleanify.exe".to_owned(),
            DRY_RUN_FLAG.to_owned()
        ]));
        assert!(!is_dry_run(["cleanify.exe".to_owned()]));
        assert!(!is_dry_run(["cleanify.exe".to_owned(), "--dry".to_owned()]));
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
