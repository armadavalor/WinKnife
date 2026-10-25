//! Domain errors and the wire format the UI receives.
//!
//! Why: commands must never leak a `Debug` blob to the WebView. Everything
//! that crosses IPC becomes an [`ApiError`] with a stable machine-readable
//! `code`, a human-readable `message` and optional `detail`
//! (ARCHITECTURE section 8).

use serde::Serialize;

/// Result alias used throughout `core` and `modules`.
pub type Result<T> = std::result::Result<T, CoreError>;

/// Everything that can go wrong inside the Rust core.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CoreError {
    /// SQLite failed.
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    /// Filesystem access failed.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON (de)serialisation failed.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// No module with this id is registered.
    #[error("unknown module: {0}")]
    UnknownModule(String),

    /// Two modules claim the same id — a programming error in the registry.
    #[error("duplicate module id: {0}")]
    DuplicateModule(String),

    /// A recommendation violated its invariants (see
    /// [`Recommendation::new`](crate::core::types::Recommendation::new)).
    #[error("invalid recommendation: {0}")]
    InvalidRecommendation(String),

    /// The application data directory could not be resolved or created.
    #[error("could not resolve the application data directory: {0}")]
    AppDataDir(String),

    /// A UI event could not be delivered to the WebView.
    #[error("could not emit event `{event}`: {source}")]
    Emit {
        /// The event name that failed, e.g. `scan:progress`.
        event: &'static str,
        /// The underlying Tauri error.
        source: tauri::Error,
    },

    /// A rule pack could not be parsed or violated the schema. Carries the
    /// pack name and the offending field so the failure is actionable
    /// (RULES-FORMAT.md: "loud failure beats silent skip").
    #[error("rule pack `{pack}` is invalid at `{field}`: {message}")]
    RulePack {
        /// File stem or declared id of the pack.
        pack: String,
        /// Dotted path to the field at fault, e.g. `rules[0].score.max_bucket`.
        field: String,
        /// What is wrong with it.
        message: String,
    },

    /// A quarantine operation failed for one item.
    #[error("quarantine error: {0}")]
    Quarantine(String),

    /// A copy into or out of quarantine did not verify byte-for-byte. The
    /// source is always left untouched when this happens.
    #[error("verification failed after copying `{0}`; the original was left in place")]
    VerifyFailed(String),

    /// A quarantine item id or batch id is not in the ledger.
    #[error("no quarantine record for `{0}`")]
    UnknownQuarantineItem(String),

    /// `quarantine_purge_items` was called without the confirmation flag the
    /// drawer's "cannot be undone" dialog sets (K-020).
    #[error("removing quarantined items for good needs the user's confirmation")]
    PurgeNotConfirmed,

    /// The OS refused a power action.
    #[error("power action failed: {0}")]
    PowerAction(String),

    /// A power action was requested from a context that must never reach the
    /// operating system (a test build). See `core::platform::power`.
    #[error("power action `{0}` blocked: this build must never touch the real power API")]
    PowerActionBlocked(&'static str),

    /// The recycle-bin shell API refused.
    #[error("recycle bin error: {0}")]
    RecycleBin(String),

    /// Emptying the recycle bin was requested from a test build.
    #[error("emptying the recycle bin is blocked: this build must never touch the real bin")]
    RecycleBinBlocked,

    /// A shutdown timer request does not make sense (past target, over the 24h
    /// limit, malformed clock time).
    #[error("invalid timer: {0}")]
    InvalidTimer(String),

    /// A scan id the UI referred to is not in the store (app restarted, or the
    /// scan was evicted).
    #[error("unknown scan: {0}")]
    UnknownScan(String),

    /// A registry read or write failed.
    #[error("registry error: {0}")]
    Registry(String),

    /// A tweak was refused or could not be applied or undone (K-018).
    #[error("tweak refused: {0}")]
    Tweak(String),

    /// A registry write was requested from a build that must never change the
    /// machine (any test binary). See `core::platform::guard`.
    #[error("registry write blocked: this build must never modify the real registry")]
    RegistryBlocked,

    /// The Task Scheduler refused, or is unavailable.
    #[error("task scheduler error: {0}")]
    TaskScheduler(String),

    /// A scheduled-task change was requested from a build that must never
    /// change the machine.
    #[error("task scheduler write blocked: this build must never modify real scheduled tasks")]
    TaskSchedulerBlocked,

    /// An autostart entry id the UI referred to is not in the last scan.
    #[error("unknown startup entry: {0}")]
    UnknownStartupEntry(String),

    /// File Explorer could not be opened.
    #[error("could not show this in File Explorer: {0}")]
    Explorer(String),

    /// A restore point could not be requested, or the settings window opened.
    #[error("restore point error: {0}")]
    RestorePoint(String),

    /// A legacy Windows panel could not be opened.
    #[error("could not open this Windows panel: {0}")]
    LegacyPanel(String),

    /// A legacy panel id the UI sent is not in the catalogue.
    #[error("unknown Windows panel: {0}")]
    UnknownLegacyPanel(String),

    /// An installed-app id the UI sent is not in the last list.
    #[error("unknown installed app: {0}")]
    UnknownApp(String),

    /// The database was written by a newer build. Opening it here could
    /// purge items the newer build promised to keep (round 3).
    #[error(
        "the WinKnife database is at schema version {found}, but this build only knows up to \
         version {known}: it was written by a newer WinKnife. Install that version again; this \
         one will not open the database, so nothing in it is changed or removed"
    )]
    SchemaTooNew { found: u32, known: u32 },

    /// A clean-up, restore or purge is already running (issue #39). A double
    /// click is refused instead of queued behind the database lock.
    #[error("{} is already running; wait for it to finish", .0.label())]
    Busy(crate::core::busy::Operation),

    /// The hosts file editor (module C4).
    #[error("hosts file: {0}")]
    Hosts(String),

    /// The repair kit (module C6) could not start or read SFC / DISM.
    #[error("repair: {0}")]
    Repair(String),
    /// Reading the socket tables failed (module C14).
    #[error("network: {0}")]
    Network(String),

    /// Windows refused the change because WinKnife runs without administrator
    /// rights (K-021). The UI offers "Restart as administrator" for this code.
    #[error("this needs administrator rights: {0}")]
    NeedsAdmin(String),

    /// A test build asked to start an elevated copy of itself; refused by the
    /// same guard as the other real-OS effects.
    #[error("starting an elevated copy is disabled in this build")]
    ElevationBlocked,
}

impl CoreError {
    /// Stable identifier the UI can branch on. Never localise this.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Db(_) => "db",
            Self::Io(e) if e.kind() == std::io::ErrorKind::PermissionDenied => "needs_admin",
            Self::Io(_) => "io",
            Self::Serde(_) => "serde",
            Self::UnknownModule(_) => "unknown_module",
            Self::DuplicateModule(_) => "duplicate_module",
            Self::InvalidRecommendation(_) => "invalid_recommendation",
            Self::AppDataDir(_) => "app_data_dir",
            Self::Emit { .. } => "emit",
            Self::RulePack { .. } => "rule_pack",
            Self::Quarantine(_) => "quarantine",
            Self::VerifyFailed(_) => "verify_failed",
            Self::UnknownQuarantineItem(_) => "unknown_quarantine_item",
            Self::PurgeNotConfirmed => "purge_not_confirmed",
            Self::PowerAction(_) => "power_action",
            Self::PowerActionBlocked(_) => "power_action_blocked",
            Self::RecycleBin(_) => "recycle_bin",
            Self::RecycleBinBlocked => "recycle_bin_blocked",
            Self::InvalidTimer(_) => "invalid_timer",
            Self::UnknownScan(_) => "unknown_scan",
            Self::Registry(_) => "registry",
            Self::RegistryBlocked => "registry_blocked",
            Self::Tweak(_) => "tweak",
            Self::TaskScheduler(_) => "task_scheduler",
            Self::TaskSchedulerBlocked => "task_scheduler_blocked",
            Self::UnknownStartupEntry(_) => "unknown_startup_entry",
            Self::Explorer(_) => "explorer",
            Self::RestorePoint(_) => "restore_point",
            Self::LegacyPanel(_) => "legacy_panel",
            Self::UnknownLegacyPanel(_) => "unknown_legacy_panel",
            Self::UnknownApp(_) => "unknown_app",
            Self::SchemaTooNew { .. } => "schema_too_new",
            Self::Busy(_) => "busy",
            Self::Hosts(_) => "hosts",
            Self::Repair(_) => "repair",
            Self::Network(_) => "network",
            Self::NeedsAdmin(_) => "needs_admin",
            Self::ElevationBlocked => "elevation_blocked",
        }
    }
}

/// The serialisable error every `#[tauri::command]` returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    /// Stable machine-readable code, e.g. `unknown_module`.
    pub code: String,
    /// Human-readable, English, safe to show in the UI.
    pub message: String,
    /// Optional extra context (a path, an underlying error string).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ApiError {
    /// Build an error from parts.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            detail: None,
        }
    }

    /// Attach extra context.
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

impl From<CoreError> for ApiError {
    fn from(err: CoreError) -> Self {
        let code = err.code();
        let message = err.to_string();
        let detail = std::error::Error::source(&err).map(ToString::to_string);
        Self {
            code: code.to_owned(),
            message,
            detail,
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_codes_are_stable() {
        assert_eq!(
            CoreError::UnknownModule("x".into()).code(),
            "unknown_module"
        );
        assert_eq!(CoreError::AppDataDir("x".into()).code(), "app_data_dir");
        assert_eq!(CoreError::PurgeNotConfirmed.code(), "purge_not_confirmed");
        assert_eq!(
            CoreError::Busy(crate::core::busy::Operation::Purge).code(),
            "busy"
        );
    }

    #[test]
    fn access_denied_maps_to_needs_admin() {
        assert_eq!(CoreError::NeedsAdmin("HKLM".into()).code(), "needs_admin");
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let api: ApiError = CoreError::Io(denied).into();
        assert_eq!(api.code, "needs_admin");
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(CoreError::Io(missing).code(), "io");
        assert_eq!(CoreError::ElevationBlocked.code(), "elevation_blocked");
    }

    #[test]
    fn core_error_converts_to_api_error() {
        let api: ApiError = CoreError::UnknownModule("cleaner".into()).into();
        assert_eq!(api.code, "unknown_module");
        assert!(api.message.contains("cleaner"));
    }

    #[test]
    fn api_error_omits_absent_detail() {
        let json = serde_json::to_string(&ApiError::new("io", "boom")).expect("serialize");
        assert_eq!(json, r#"{"code":"io","message":"boom"}"#);
    }

    #[test]
    fn api_error_serialises_detail_when_present() {
        let json = serde_json::to_string(&ApiError::new("io", "boom").with_detail("C:\\x"))
            .expect("serialize");
        assert!(json.contains(r#""detail":"C:\\x""#), "got {json}");
    }
}
