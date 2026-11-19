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
import { applyTheme } from "$lib/theme.svelte";

applyTheme();

const app = mount(App, { target: document.getElementById("app")! });

export default app;
