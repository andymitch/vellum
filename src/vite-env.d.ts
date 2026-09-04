/// <reference types="svelte" />
/// <reference types="vite/client" />

/// Version of the running build, substituted by Vite (see vite.config.ts).
/// Only the web build reads it; the Tauri builds ask their backend.
declare const __APP_VERSION__: string;
