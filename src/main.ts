// Must run before any preference store evaluates (they read localStorage at
// module-eval). Keep this the first import. See migrate-prefs.ts.
import "$lib/migrate-prefs";
import { mount } from "svelte";
import "@fontsource/fraunces/400.css";
import "@fontsource/fraunces/600.css";
import "@fontsource/fraunces/700.css";
import "@fontsource/space-mono/400.css";
import "@fontsource/space-mono/700.css";
import "@fontsource/inter/400.css";
import "@fontsource/inter/600.css";
import "@fontsource/inter/700.css";
import "@fontsource/lora/400.css";
import "@fontsource/lora/600.css";
import "@fontsource/lora/700.css";
import "./app.css";
import App from "./App.svelte";
import Popup from "./Popup.svelte";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isTauri } from "$lib/platform";
import { applyTheme, useSystemTheme } from "$lib/theme.svelte";

// The menu-bar popup (#288) is a second window over the same bundle.
const popup = isTauri && getCurrentWindow().label === "popup";
if (popup) useSystemTheme();
else applyTheme();
const app = mount(popup ? Popup : App, { target: document.getElementById("app")! });

export default app;
