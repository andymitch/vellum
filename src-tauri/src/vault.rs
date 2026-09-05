//! The vault lives in `crates/vellum-vault` so the browser shell can compile the
//! same code to wasm. This re-export keeps `crate::vault::…` working for the
//! desktop shell's callers (`lib.rs`, `mcp.rs`, `link.rs`, `share.rs`).
pub use vellum_vault::vault::*;
