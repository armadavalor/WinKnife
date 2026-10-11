//! Rolling audit log (ARCHITECTURE section 8).
//!
//! Why a file log in a desktop app: this one deletes things. When a user asks
//! "where did my file go", the answer has to be a line in a log with a path, a
//! batch id and a timestamp — not a shrug. Every executed item, every restore
//! and every purge writes one.
//!
//! The log lives in `{app_data}/logs/`, rotates daily, keeps the newest
//! [`MAX_LOG_FILES`] files, and keeps its own directory out of the cleaner's
//! reach via the protected-path list.

use std::path::{Path, PathBuf};

use tracing_appender::rolling::{InitError, RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Directory name under the app data dir.
pub const LOG_DIR_NAME: &str = "logs";

/// Prefix of the rotating log files, e.g. `winknife.log.2026-08-06`.
pub const LOG_FILE_PREFIX: &str = "winknife.log";

/// How many daily log files to keep: about two weeks of use.
///
/// The appender counts files, not days. When it opens, and at each daily
/// rotation, it deletes the oldest `winknife.log*` files in this directory
/// until no more than this many remain, the current one included. That bounds
/// both disk use and how long the local paths of cleaned items stay on disk.
/// The files are the app's own, never user data.
pub const MAX_LOG_FILES: usize = 14;

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

    // `rolling::daily` panics when the file cannot be opened; the builder
    // hands the error back instead.
    let appender = match build_appender(&dir) {
        Ok(appender) => appender,
        Err(err) => {
            eprintln!("winknife: cannot open the log file: {err}");
            return None;
        }
    };
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

/// The daily file appender, pruned down to [`MAX_LOG_FILES`] as it opens.
fn build_appender(dir: &Path) -> Result<RollingFileAppender, InitError> {
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .max_log_files(MAX_LOG_FILES)
        .build(dir)
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

    #[test]
    fn opening_the_log_prunes_old_files_down_to_the_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        for day in 1..=MAX_LOG_FILES + 6 {
            let name = format!("{LOG_FILE_PREFIX}.2000-01-{day:02}");
            std::fs::write(dir.path().join(name), "old").expect("seed log");
        }
        let unrelated = dir.path().join("notes.txt");
        std::fs::write(&unrelated, "keep").expect("seed unrelated file");

        let appender = build_appender(dir.path()).expect("appender");
        let logs = std::fs::read_dir(dir.path())
            .expect("read the log dir")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(LOG_FILE_PREFIX)
            })
            .count();
        drop(appender);

        // 20 seeded: the 7 oldest go, 13 stay, plus today's new file.
        assert_eq!(logs, MAX_LOG_FILES);
        assert!(
            unrelated.is_file(),
            "pruning touched a file it does not own"
        );
    }
}
