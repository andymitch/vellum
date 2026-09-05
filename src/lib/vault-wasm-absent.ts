// Stands in for `vault-wasm.ts` in the Tauri build.
//
// `vault.ts` imports both backends and picks between them at runtime, which is
// what keeps the `VaultBackend` check honest — a command added to one and
// forgotten in the other does not compile. But a static import means the
// bundler keeps both, and the wasm backend drags in a ~10 MB WebAssembly module
// and a worker that the desktop and Android apps can never reach: `isTauri` is
// true there, so the wasm branch is dead.
//
// Vite aliases this file in for the non-web build (see vite.config.ts). Nothing
// reads its exports — the module object is only ever the unselected branch of a
// ternary — so it is deliberately empty rather than a set of throwing stubs.
// Type-checking still sees the real module, so the interface stays enforced.
export {};
