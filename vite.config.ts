import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import { VitePWA } from "vite-plugin-pwa";
import path from "node:path";
import process from "node:process";

const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
//
// Two targets, one bundle. The default build is what Tauri wraps; `--mode web`
// (bun run build:web) is the hosted build published to GitHub Pages and
// installed as a PWA on iOS (#221/#222). The difference is only in packaging:
// where it's served from, and the service worker + manifest that make it
// installable. Which vault backend runs is decided at runtime — see platform.ts.
export default defineConfig(({ mode }) => {
  const web = mode === "web";
  return {
    plugins: [
      tailwindcss(),
      svelte(),
      // Only the hosted build gets a service worker: inside Tauri the app is
      // already local, and a worker caching tauri://localhost would just add a
      // stale layer between the app and its own assets.
      ...(web
        ? [
            VitePWA({
              // The worker fetches a new build in the background and it applies
              // on the next load, so the web app has no "check for updates" —
              // see the version line in SettingsSheet.
              registerType: "autoUpdate",
              // Registration is injected into index.html by the plugin, so no
              // PWA-only import leaks into the app's entry point.
              injectRegister: "script-defer",
              includeAssets: ["favicon.png", "apple-touch-icon.png"],
              manifest: {
                name: "Vellum",
                short_name: "Vellum",
                description: "A local-first Markdown notes app.",
                // Relative to the manifest, which sits at `base` — so the same
                // manifest works whatever path the site is published under.
                start_url: ".",
                scope: ".",
                display: "standalone",
                orientation: "any",
                // Light "Things" palette; the running app keeps a theme-color
                // meta in step with whichever palette is chosen (theme.svelte.ts).
                background_color: "#ffffff",
                theme_color: "#ffffff",
                icons: [
                  { src: "pwa-192.png", sizes: "192x192", type: "image/png" },
                  { src: "pwa-512.png", sizes: "512x512", type: "image/png" },
                  {
                    src: "pwa-maskable-512.png",
                    sizes: "512x512",
                    type: "image/png",
                    purpose: "maskable",
                  },
                ],
              },
              workbox: {
                // Precache the shell — markup, styles, fonts, icons — and let
                // scripts be cached as they are fetched. Precaching the lot
                // means a ~7 MB install on first visit, most of it mermaid,
                // katex and cytoscape chunks that a note only pulls in if it
                // actually contains a diagram. Everything the app has loaded
                // once is then available offline; a diagram in a note you have
                // never opened offline is the one thing that isn't.
                globPatterns: ["**/*.{css,html,png,svg,webmanifest,woff,woff2}"],
                runtimeCaching: [
                  {
                    urlPattern: ({ url, request }) =>
                      url.origin === self.location.origin && request.destination === "script",
                    handler: "StaleWhileRevalidate",
                    options: {
                      cacheName: "vellum-scripts",
                      // Chunk names are content-hashed, so superseded entries
                      // would otherwise pile up forever.
                      expiration: { maxEntries: 120, maxAgeSeconds: 60 * 60 * 24 * 60 },
                    },
                  },
                ],
                cleanupOutdatedCaches: true,
                navigateFallback: "index.html",
              },
            }),
          ]
        : []),
    ],

    // GitHub Pages serves the app from a subdirectory (/vellum/app/), so the
    // Pages workflow passes it in. Tauri loads from the root of its own
    // protocol and must keep "/".
    base: web ? (process.env.VELLUM_BASE ?? "/") : "/",

    define: {
      // The version the web build reports (platform.ts). Tauri builds ask the
      // backend instead, which knows the version stamped at release time.
      __APP_VERSION__: JSON.stringify(process.env.VELLUM_VERSION ?? "dev"),
    },

    resolve: {
      alias: {
        $lib: path.resolve("./src/lib"),
      },
    },

    build: {
      // Tauri serves the static build from ../build (frontendDist); the hosted
      // build goes elsewhere so the two never overwrite each other.
      outDir: web ? "build-web" : "build",
    },

    // Vite options tailored for Tauri development
    clearScreen: false,
    server: {
      port: 1420,
      strictPort: true,
      // Bind all interfaces so one server serves both the desktop webview
      // (localhost) and a mobile device/emulator (LAN IP) simultaneously.
      host: "0.0.0.0",
      hmr: host
        ? {
            protocol: "ws",
            host,
            port: 1421,
          }
        : undefined,
      watch: {
        // tell Vite to ignore watching `src-tauri`
        ignored: ["**/src-tauri/**"],
      },
    },
  };
});
