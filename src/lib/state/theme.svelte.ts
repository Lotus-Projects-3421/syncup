export type ThemeMode = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";

const STORAGE_KEY = "syncup.theme-mode";
const DARK_MEDIA_QUERY = "(prefers-color-scheme: dark)";

function isThemeMode(value: string | null): value is ThemeMode {
  return value === "light" || value === "dark" || value === "system";
}

function readStoredMode(): ThemeMode {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    return isThemeMode(stored) ? stored : "system";
  } catch {
    console.warn("Stored theme preference could not be read; using the system theme.");
    return "system";
  }
}

function systemTheme(): ResolvedTheme {
  return window.matchMedia(DARK_MEDIA_QUERY).matches ? "dark" : "light";
}

class ThemeState {
  mode = $state<ThemeMode>(readStoredMode());
  resolved = $state<ResolvedTheme>(systemTheme());

  constructor() {
    this.apply();
    window.matchMedia(DARK_MEDIA_QUERY).addEventListener("change", this.handleSystemChange);
  }

  setMode(mode: ThemeMode): void {
    this.mode = mode;
    try {
      localStorage.setItem(STORAGE_KEY, mode);
    } catch {
      console.warn("Theme preference could not be saved; it will reset on restart.");
    }
    this.apply();
  }

  private apply(): void {
    this.resolved = this.mode === "system" ? systemTheme() : this.mode;
    document.documentElement.classList.toggle("dark", this.resolved === "dark");
    document.documentElement.style.colorScheme = this.resolved;
  }

  private handleSystemChange = (): void => {
    if (this.mode === "system") {
      this.apply();
    }
  };
}

export const theme = new ThemeState();
