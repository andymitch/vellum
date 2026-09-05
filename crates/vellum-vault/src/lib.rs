//! The vault, shell-free.
//!
//! `vault` is a module rather than the crate root because `#[tauri::command]`
//! emits `#[macro_export]` macros: at the root, the local definition and the
//! re-export at the crate root are the same name in the same namespace, and the
//! crate will not compile.
pub mod vault;
