//! Rolling audit log (ARCHITECTURE section 8).
//!
//! Why a file log in a desktop app: this one deletes things. When a user asks
//! "where did my file go", the answer has to be a line in a log with a path, a
//! batch id and a timestamp — not a shrug. Every executed item, every restore
//! and every purge writes one.
//!
//! The log lives in `{app_data}/logs/`, rotates daily, and keeps its own
//! directory out of the cleaner's reach via the protected-path list.

use std::path::{Path, PathBuf};

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Directory name under the app data dir.
pub const LOG_DIR_NAME: &str = "logs";

/// Prefix of the rotating log files, e.g. `winknife.log.2026-08-06`.
pub const LOG_FILE_PREFIX: &str = "winknife.log";

/// Environment variable that overrides the default filter.
pub const FILTER_ENV: &str = "WINKNIFE_LOG";

/// Where the logs live for a given app data directory.
pub fn log_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(LOG_DIR_NAME)
}

/// Install the global subscriber: a rolling file appender plus stderr.
///
/// Returns the appender guard, which must stay alive for the life of the
/// process or buffered lines are lost on exit.
///
/// Called once from `run()`. A second call is ignored rather than fatal, since
/// failing to log is never a reason to refuse to start.
pub fn init(app_data_dir: &Path) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let dir = log_dir(app_data_dir);
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("winknife: cannot create the log directory: {err}");
        return None;
    }

    let appender = tracing_appender::rolling::daily(&dir, LOG_FILE_PREFIX);
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let filter = EnvFilter::try_from_env(FILTER_ENV)
        .unwrap_or_else(|_| EnvFilter::new("info,winknife_lib=debug"));

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true);
    let console_layer = tracing_subscriber::fmt::layer().with_target(false);

    let installed = tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(console_layer)
        .try_init();

    match installed {
        Ok(()) => {
            tracing::info!(dir = %dir.display(), "audit log started");
            Some(guard)
        }
        Err(err) => {
            eprintln!("winknife: tracing was already initialised: {err}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_live_beside_the_database() {
        assert_eq!(
            log_dir(Path::new("C:\\data\\WinKnife")),
            PathBuf::from("C:\\data\\WinKnife\\logs")
        );
    }

    #[test]
    fn init_creates_the_directory_and_hands_back_a_guard() {
        let dir = tempfile::tempdir().expect("tempdir");
        // The global subscriber may already be installed by another test in
        // this binary; either way the directory has to appear.
        let _guard = init(dir.path());
        assert!(log_dir(dir.path()).is_dir());
    }

    #[test]
    fn the_filter_environment_variable_is_namespaced() {
        assert!(FILTER_ENV.starts_with("WINKNIFE_"));
    }
}
