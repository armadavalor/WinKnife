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
}

impl CoreError {
    /// Stable identifier the UI can branch on. Never localise this.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Db(_) => "db",
            Self::Io(_) => "io",
            Self::Serde(_) => "serde",
            Self::UnknownModule(_) => "unknown_module",
            Self::DuplicateModule(_) => "duplicate_module",
            Self::InvalidRecommendation(_) => "invalid_recommendation",
            Self::AppDataDir(_) => "app_data_dir",
            Self::Emit { .. } => "emit",
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
