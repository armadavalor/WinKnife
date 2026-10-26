//! The module contract and the registry that owns every tool.
//!
//! Why: the whole app is "one trait, many tools". A new tool is a file in
//! `modules/`, a route folder in `src/routes/` and one line in the
//! `register_modules!` block — the sidebar, manifests and routing follow
//! automatically (ARCHITECTURE section 2).

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::brain::Brain;
use crate::core::error::{CoreError, Result};
use crate::core::types::{ExecReport, Finding, Recommendation};

/// What a module can do, so the UI can hide what does not apply.
///
/// The shutdown timer, for example, has no scan step at all and drives its own
/// commands instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleCapabilities {
    /// Module produces findings via `scan`.
    pub has_scan: bool,
    /// Module can apply approved recommendations via `execute`.
    pub has_execute: bool,
    /// Module stores per-module settings.
    pub has_settings: bool,
}

impl ModuleCapabilities {
    /// A file-oriented module: scans, executes, has settings.
    pub const FULL: Self = Self {
        has_scan: true,
        has_execute: true,
        has_settings: true,
    };

    /// A module that does none of the three (the default for utilities that
    /// only expose their own commands).
    pub const NONE: Self = Self {
        has_scan: false,
        has_execute: false,
        has_settings: false,
    };
}

impl Default for ModuleCapabilities {
    fn default() -> Self {
        Self::NONE
    }
}

/// The sidebar tab a tool is listed under. Every manifest names one, so a new
/// tool cannot be added without choosing where it shows up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModuleCategory {
    /// Removing junk: caches, duplicates, empty folders.
    Cleanup,
    /// Looking at and handling files on disk.
    Files,
    /// Windows settings, repair and restore points.
    System,
    /// Installed programs and what starts with Windows.
    Apps,
    /// Network state and the hosts file.
    Network,
    /// Shutdown timer and keeping the PC awake.
    Power,
    /// Anything that fits none of the above.
    Other,
}

/// Everything the UI needs to render a module in the sidebar without knowing
/// anything else about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleManifest {
    /// kebab-case, stable, also the UI route segment (`/dummy`).
    pub id: String,
    /// English display name. The UI may override it from the i18n dictionary.
    pub name: String,
    /// One-line English description shown on the dashboard card.
    pub description: String,
    /// Icon name the UI maps to an inline SVG.
    pub icon: String,
    /// The sidebar tab the tool is listed under.
    pub category: ModuleCategory,
    /// What this module supports.
    pub capabilities: ModuleCapabilities,
}

/// Read-only context for a scan run.
///
/// Phase 1 extends this with the scan roots, a cancellation token and the
/// handle used to emit `scan:progress`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ScanContext {
    /// Id of this scan; echoed in every event and stored in `scans`.
    pub scan_id: Uuid,
    /// Id of the module being scanned.
    pub module_id: String,
}

impl ScanContext {
    /// Start a context for a fresh scan.
    pub fn new(module_id: impl Into<String>) -> Self {
        Self {
            scan_id: Uuid::new_v4(),
            module_id: module_id.into(),
        }
    }
}

/// Context for applying approved recommendations.
///
/// Phase 1 extends this with the quarantine handle and the executor's logger.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ExecContext {
    /// The scan whose recommendations are being applied.
    pub scan_id: Uuid,
    /// Id of the module doing the work.
    pub module_id: String,
}

impl ExecContext {
    /// Start a context for applying the results of `scan_id`.
    pub fn new(scan_id: Uuid, module_id: impl Into<String>) -> Self {
        Self {
            scan_id,
            module_id: module_id.into(),
        }
    }
}

/// The contract every tool implements.
///
/// `scan` and `analyze` are **read-only**: they must not touch the filesystem
/// or the registry. Only `execute` changes anything, and it does so through the
/// executor and quarantine — never with a direct delete.
///
/// Modules that are not about files (the shutdown timer) leave `scan`,
/// `analyze` and `execute` at their no-op defaults and expose module-specific
/// commands instead; the trait still gives them registry presence, a manifest
/// and settings storage.
pub trait ToolModule: Send + Sync {
    /// Stable kebab-case id. Must match `manifest().id`.
    fn id(&self) -> &'static str;

    /// Everything the UI needs to render this module.
    fn manifest(&self) -> ModuleManifest;

    /// Collect findings. Read-only; emits progress events.
    ///
    /// # Errors
    /// Returns an error if the scan could not be completed at all. Per-item
    /// problems are reported as findings or skipped, not as errors.
    fn scan(&self, _ctx: &ScanContext) -> Result<Vec<Finding>> {
        Ok(Vec::new())
    }

    /// Turn findings into scored, explained recommendations. Read-only.
    fn analyze(&self, _findings: &[Finding], _brain: &Brain) -> Vec<Recommendation> {
        Vec::new()
    }

    /// Apply the recommendations the user approved, by id.
    ///
    /// # Errors
    /// Returns an error only when the batch could not be started. Individual
    /// item failures belong in the returned [`ExecReport`].
    fn execute(&self, _approved: &[Uuid], _ctx: &ExecContext) -> Result<ExecReport> {
        Ok(ExecReport::default())
    }
}

/// Owns every registered module and hands out manifests to the UI.
#[derive(Clone)]
pub struct ModuleRegistry {
    modules: Vec<Arc<dyn ToolModule>>,
}

impl ModuleRegistry {
    /// Build a registry, rejecting duplicate or malformed ids.
    ///
    /// # Errors
    /// Returns [`CoreError::DuplicateModule`] if two modules share an id.
    pub fn new(modules: Vec<Arc<dyn ToolModule>>) -> Result<Self> {
        let mut seen = BTreeSet::new();
        for module in &modules {
            let id = module.id();
            if !seen.insert(id) {
                return Err(CoreError::DuplicateModule(id.to_owned()));
            }
        }
        Ok(Self { modules })
    }

    /// Manifests in registration order; this is the sidebar order.
    pub fn manifests(&self) -> Vec<ModuleManifest> {
        self.modules.iter().map(|m| m.manifest()).collect()
    }

    /// Look a module up by id.
    ///
    /// # Errors
    /// Returns [`CoreError::UnknownModule`] when nothing matches.
    pub fn get(&self, id: &str) -> Result<Arc<dyn ToolModule>> {
        self.modules
            .iter()
            .find(|m| m.id() == id)
            .cloned()
            .ok_or_else(|| CoreError::UnknownModule(id.to_owned()))
    }

    /// Number of registered modules.
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    /// Whether nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// Iterate over the registered modules.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn ToolModule>> {
        self.modules.iter()
    }
}

impl std::fmt::Debug for ModuleRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleRegistry")
            .field(
                "ids",
                &self.modules.iter().map(|m| m.id()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct Stub(&'static str);

    impl ToolModule for Stub {
        fn id(&self) -> &'static str {
            self.0
        }

        fn manifest(&self) -> ModuleManifest {
            ModuleManifest {
                id: self.0.to_owned(),
                name: "Stub".to_owned(),
                description: "test double".to_owned(),
                icon: "beaker".to_owned(),
                category: ModuleCategory::Other,
                capabilities: ModuleCapabilities::NONE,
            }
        }
    }

    fn registry(ids: &[&'static str]) -> Result<ModuleRegistry> {
        let modules = ids
            .iter()
            .map(|id| Arc::new(Stub(id)) as Arc<dyn ToolModule>)
            .collect();
        ModuleRegistry::new(modules)
    }

    #[test]
    fn registry_rejects_duplicate_ids() {
        let err = registry(&["a", "a"]).expect_err("duplicate ids must be rejected");
        assert!(matches!(err, CoreError::DuplicateModule(id) if id == "a"));
    }

    #[test]
    fn registry_preserves_registration_order() {
        let reg = registry(&["b", "a", "c"]).expect("valid registry");
        let ids: Vec<_> = reg.manifests().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["b", "a", "c"]);
    }

    #[test]
    fn registry_looks_modules_up_by_id() {
        let reg = registry(&["a", "b"]).expect("valid registry");
        assert_eq!(reg.get("b").expect("module b").id(), "b");
        // `Arc<dyn ToolModule>` is not `Debug`, so `expect_err` is unavailable.
        let err = reg.get("nope").err().expect("unknown id must error");
        assert!(matches!(err, CoreError::UnknownModule(id) if id == "nope"));
    }

    #[test]
    fn trait_defaults_are_no_ops() {
        let stub = Stub("a");
        let ctx = ScanContext::new("a");
        assert!(stub.scan(&ctx).expect("default scan").is_empty());
        assert!(stub.analyze(&[], &Brain::new()).is_empty());
        let report = stub
            .execute(&[], &ExecContext::new(ctx.scan_id, "a"))
            .expect("default execute");
        assert_eq!(report, ExecReport::default());
    }

    #[test]
    fn capability_presets_are_what_they_say() {
        const {
            assert!(ModuleCapabilities::FULL.has_scan);
            assert!(ModuleCapabilities::FULL.has_execute);
            assert!(ModuleCapabilities::FULL.has_settings);
            assert!(!ModuleCapabilities::NONE.has_scan);
            assert!(!ModuleCapabilities::NONE.has_execute);
            assert!(!ModuleCapabilities::NONE.has_settings);
        }
        assert_eq!(ModuleCapabilities::default(), ModuleCapabilities::NONE);
    }
}
