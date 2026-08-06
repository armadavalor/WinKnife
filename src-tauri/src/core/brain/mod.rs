//! Rule engine, scoring and protected paths (ARCHITECTURE section 4).
//!
//! **Phase 0 placeholder.** [`Brain`] exists only so that
//! [`ToolModule::analyze`](crate::core::module::ToolModule::analyze) can carry
//! the signature the architecture specifies without a breaking change later.
//! It holds no rules, no protected-path list and no scoring — those arrive in
//! phase 1 (hard gates) and phase 2 (scoring pipeline).

/// Shared analysis context handed to every module's `analyze`.
///
/// Empty on purpose: see the module header.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Brain {}

impl Brain {
    /// Create the (currently empty) brain.
    pub fn new() -> Self {
        Self {}
    }
}
