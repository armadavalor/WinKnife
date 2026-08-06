//! `#[tauri::command]` entry points.
//!
//! Deliberately thin: no filesystem, no registry, no SQL here — commands only
//! translate between IPC and `core`/`modules` and map errors to [`ApiError`]
//! (CONVENTIONS.md).

use tauri::State;

use crate::core::error::ApiError;
use crate::core::module::{ModuleManifest, ModuleRegistry};

/// Manifests of every registered module, in sidebar order.
///
/// # Errors
/// Currently infallible, but keeps the `Result` shape every command shares so
/// the UI's error handling does not change when it grows one.
#[tauri::command]
pub fn list_modules(registry: State<'_, ModuleRegistry>) -> Result<Vec<ModuleManifest>, ApiError> {
    Ok(registry.manifests())
}

#[cfg(test)]
mod tests {
    use crate::modules;

    #[test]
    fn list_modules_returns_the_registry_contents() {
        // The command body is `registry.manifests()`; `State` cannot be built
        // outside a running app, so the delegation target is tested directly.
        let registry = modules::registry().expect("registry");
        let manifests = registry.manifests();

        assert_eq!(manifests.len(), registry.len());
        assert!(manifests.iter().any(|m| m.id == "dummy"));
    }

    #[test]
    fn manifests_serialise_for_the_webview() {
        let manifests = modules::registry().expect("registry").manifests();
        let json = serde_json::to_value(&manifests).expect("serialize");
        let first = json.get(0).expect("at least one module");

        assert!(first.get("id").is_some(), "got {json}");
        assert!(
            first
                .get("capabilities")
                .and_then(|c| c.get("hasScan"))
                .is_some(),
            "capabilities must be camelCase, got {json}"
        );
    }
}
