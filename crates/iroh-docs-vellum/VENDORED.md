# iroh-docs 0.101.0, vendored

Not our code. This is `iroh-docs` 0.101.0 exactly as published to crates.io,
plus one addition, so that the browser build can keep a vault's replica in OPFS.

## The change

`src/store/fs.rs` gains a public constructor:

```rust
pub fn with_backend(backend: impl redb::StorageBackend) -> Result<Self> {
    let db = Database::builder().create_with_backend(backend)?;
    Self::new_impl(db)          // already exists; only ever was private
}
```

Eleven lines, purely additive. Nothing else differs from the published crate.

## Why it has to exist

`Store` offers exactly two constructors: `memory()`, backed by a `Vec<u8>` we
cannot persist or read back out, and `persistent(path)`, which needs a
filesystem. A browser has neither — `std::fs` does not exist on
`wasm32-unknown-unknown` at all, so no filesystem shim rescues `persistent`, and
`memory()` cannot be snapshotted because `InMemoryBackend`'s bytes are
unreachable. redb *does* accept an arbitrary `StorageBackend`, and OPFS sync
access handles supply the synchronous positional reads and writes redb wants —
but `new_impl`, the one function that takes a `Database`, is private.

## Why vendored rather than forked

A fork would make vellum's diff one line instead of 56 files, but it is a second
repository the build depends on forever, and we are not currently planning to
upstream this. Vendored, `cargo build` works from a clean clone with no script,
no codegen step and no network beyond crates.io.

## Scope

Only `crates/vellum-wasm` patches to this copy. The desktop and Android apps
build against stock crates.io `iroh-docs`, and never compile this directory.

## Re-vendoring on an iroh bump

Not scripted on purpose — it is a rare, deliberate act, and a build step that
fetches and mutates source is worse than a paragraph:

1. `curl -L https://static.crates.io/crates/iroh-docs/iroh-docs-<v>.crate | tar xz`
2. Replace this directory with the result.
3. Re-apply the constructor above, and delete `Cargo.toml.orig`,
   `.cargo_vcs_info.json`, `Cargo.lock`, `Makefile.toml`, `cliff.toml`.
4. Check `Self::new_impl` still exists and still takes a `redb::Database`. If
   upstream has since exposed a backend constructor of its own, delete this
   directory and use it.
