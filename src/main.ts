import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";
import { commands } from "$lib/ipc";
import { theme, type ThemeMode } from "$lib/state/theme.svelte";

const target = document.getElementById("app");

if (!target) {
  throw new Error("Application mount point #app is missing from index.html");
}

// Narrowing from the guard above does not survive into the nested bootstrap
// function, so capture an explicitly non-null target for the mount call.
const mountTarget: HTMLElement = target;

function isThemeMode(value: string): value is ThemeMode {
  return value === "light" || value === "dark" || value === "system";
}

/**
 * Reconciles the persisted backend theme before the first render so the app
 * never paints one theme and then switches to another. The theme store has
 * already applied the locally cached preference; if the backend is unavailable
 * we keep that value rather than blocking startup.
 */
async function bootstrap() {
  try {
    const settings = await commands.getSettings();
    if (isThemeMode(settings.theme) && settings.theme !== theme.mode) {
      theme.setMode(settings.theme);
    }
  } catch {
    // Backend not ready yet: keep the locally cached theme for this session.
  }
  return mount(App, { target: mountTarget });
}

const app = bootstrap();

export default app;
