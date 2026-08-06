//! The tool modules and the single list that registers them.
//!
//! Adding a tool is **2 files + 1 registry line**:
//!
//! 1. `src-tauri/src/modules/<id>.rs` implementing
//!    [`ToolModule`](crate::core::module::ToolModule) on a `Default` type.
//! 2. `src/routes/<id>/+page.svelte` for its UI.
//! 3. One line in the [`register_modules!`] block below.
//!
//! Nothing else changes: the sidebar, the dashboard and routing all read the
//! registry.

use std::sync::Arc;

use crate::core::error::Result;
use crate::core::module::{ModuleRegistry, ToolModule};

/// Declare the module files and build the registry list from one place.
///
/// Each entry is `file_stem => Type`; the type must implement `Default` and
/// [`ToolModule`]. The macro emits the `pub mod` declarations so a new tool
/// really is a single line here.
macro_rules! register_modules {
    ($($file:ident => $module:ty),+ $(,)?) => {
        $(pub mod $file;)+

        /// Every module, in sidebar order.
        fn boxed_modules() -> Vec<Arc<dyn ToolModule>> {
            vec![$(Arc::new(<$module>::default()) as Arc<dyn ToolModule>),+]
        }
    };
}

register_modules! {
    dummy => dummy::DummyModule,
}

/// Build the application's module registry.
///
/// # Errors
/// Returns [`CoreError::DuplicateModule`](crate::core::error::CoreError::DuplicateModule)
/// if two modules claim the same id.
pub fn registry() -> Result<ModuleRegistry> {
    ModuleRegistry::new(boxed_modules())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_builds_without_duplicate_ids() {
        let registry = registry().expect("registry must build");
        assert!(
            !registry.is_empty(),
            "at least the dummy module is expected"
        );
    }

    #[test]
    fn every_module_id_is_kebab_case() {
        for module in registry().expect("registry").iter() {
            let id = module.id();
            assert!(!id.is_empty(), "module id must not be empty");
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "module id `{id}` is not kebab-case"
            );
            assert!(
                !id.starts_with('-') && !id.ends_with('-'),
                "module id `{id}` must not start or end with a dash"
            );
        }
    }

    #[test]
    fn every_manifest_is_complete() {
        for manifest in registry().expect("registry").manifests() {
            assert!(
                !manifest.name.trim().is_empty(),
                "{} has no name",
                manifest.id
            );
            assert!(
                !manifest.description.trim().is_empty(),
                "{} has no description",
                manifest.id
            );
            assert!(
                !manifest.icon.trim().is_empty(),
                "{} has no icon",
                manifest.id
            );
        }
    }

    #[test]
    fn manifest_ids_match_their_module_ids() {
        for module in registry().expect("registry").iter() {
            assert_eq!(module.manifest().id, module.id());
        }
    }

    #[test]
    fn the_dummy_module_is_registered() {
        let registry = registry().expect("registry");
        assert_eq!(registry.get("dummy").expect("dummy module").id(), "dummy");
    }
}
