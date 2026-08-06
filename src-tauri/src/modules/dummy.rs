//! Dummy module — proves the registry wiring end to end and nothing else.
//!
//! Why it exists: phase 0 ships zero features, but the sidebar, the manifest
//! contract and `list_modules` still have to be demonstrably real. Delete this
//! once a second genuine module exists.

use crate::core::module::{ModuleCapabilities, ModuleManifest, ToolModule};

/// A module that scans nothing, recommends nothing and executes nothing.
#[derive(Debug, Default)]
pub struct DummyModule;

impl ToolModule for DummyModule {
    fn id(&self) -> &'static str {
        "dummy"
    }

    fn manifest(&self) -> ModuleManifest {
        ModuleManifest {
            id: self.id().to_owned(),
            name: "Dummy".to_owned(),
            description: "Placeholder tool proving the module registry. Does nothing.".to_owned(),
            icon: "beaker".to_owned(),
            capabilities: ModuleCapabilities::NONE,
        }
    }

    // scan / analyze / execute stay at the trait's no-op defaults.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::brain::Brain;
    use crate::core::module::{ExecContext, ScanContext};
    use crate::core::types::ExecReport;

    #[test]
    fn manifest_id_matches_the_trait_id() {
        let module = DummyModule;
        assert_eq!(module.manifest().id, module.id());
    }

    #[test]
    fn dummy_module_does_nothing() {
        let module = DummyModule;
        let ctx = ScanContext::new(module.id());

        assert!(module.scan(&ctx).expect("scan").is_empty());
        assert!(module.analyze(&[], &Brain::new()).is_empty());
        assert_eq!(
            module
                .execute(&[], &ExecContext::new(ctx.scan_id, module.id()))
                .expect("execute"),
            ExecReport::default()
        );
    }

    #[test]
    fn dummy_module_declares_no_capabilities() {
        assert_eq!(
            DummyModule.manifest().capabilities,
            ModuleCapabilities::NONE
        );
    }
}
