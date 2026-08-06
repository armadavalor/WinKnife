//! Everything the WebView is not allowed to do: filesystem, registry, database.
//!
//! `commands.rs` stays a thin translation layer; all real work lives here and
//! in `modules/` (CONVENTIONS.md).

pub mod brain;
pub mod db;
pub mod error;
pub mod events;
pub mod executor;
pub mod logging;
pub mod module;
pub mod platform;
pub mod quarantine;
pub mod scanner;
pub mod settings;
pub mod state;
#[cfg(test)]
pub(crate) mod test_support;
pub mod types;

pub use error::{ApiError, CoreError, Result};
