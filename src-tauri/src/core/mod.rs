//! Everything the WebView is not allowed to do: filesystem, registry, database.
//!
//! `commands.rs` stays a thin translation layer; all real work lives here and
//! in `modules/` (CONVENTIONS.md).

pub mod db;
pub mod error;
pub mod types;

pub use error::{ApiError, CoreError, Result};
