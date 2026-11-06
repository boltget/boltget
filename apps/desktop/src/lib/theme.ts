// The app's theme at runtime: the Theme setting (System, Light or Dark),
// applied through color-scheme; native code is told the resolved theme so
// the window background and border match. The setting lives with the other
// settings in the backend, which answers after the first frame, so the page
// also keeps the last one in localStorage and starts with it (no flash of
// the wrong theme on a start).
import {
  applyTheme,
  resolveTheme,
  systemPrefersDark,
  watchSystemTheme,
  type Theme,
  type ThemeSetting,
} from "@boltget/ui/theme";
import { setWindowChrome } from "./native";

const CACHE_KEY = "boltget.theme";

let setting: ThemeSetting = "system";
let systemDark = systemPrefersDark();
/** A theme forced for review (BOLTGET_DEV_THEME, `?theme=`, Ctrl+Shift+L). */
let forced: ThemeSetting | null = null;

export function currentTheme(): Theme {
  return resolveTheme(forced ?? setting, systemDark);
}

async function sync(): Promise<void> {
  applyTheme(document.documentElement, forced ?? setting);
  await setWindowChrome(currentTheme());
}

/** The theme the last run used, for the first frame. */
export function cachedThemeSetting(): ThemeSetting {
  try {
    return parseThemeSetting(localStorage.getItem(CACHE_KEY));
  } catch {
    return "system";
  }
}

/**
 * Applies the theme and keeps it in sync with the OS. `dev` is a theme
 * forced for review in debug builds; it wins over the setting.
 */
export async function startTheme(initial: ThemeSetting, dev: ThemeSetting | null): Promise<void> {
  setting = initial;
  forced = dev;
  watchSystemTheme((dark) => {
    systemDark = dark;
    void sync();
  });
  await sync();
}

/** The Theme setting changed (or arrived from the backend). */
export function setThemeSetting(next: ThemeSetting): void {
  try {
    localStorage.setItem(CACHE_KEY, next);
  } catch {
    // The cache only avoids a flash on the next start.
  }
  if (next === setting) return;
  setting = next;
  void sync();
}

/**
 * Review aid for debug builds only: Ctrl+Shift+L flips between light and
 * dark, overriding the setting until the app restarts.
 */
export function enableThemeToggle(): void {
  addEventListener("keydown", (event) => {
    if (!(event.ctrlKey && event.shiftKey && event.code === "KeyL")) return;
    event.preventDefault();
    forced = currentTheme() === "dark" ? "light" : "dark";
    void sync();
  });
}

/** Reads a theme setting; anything unknown is System. */
export function parseThemeSetting(value: string | null): ThemeSetting {
  return value === "light" || value === "dark" ? value : "system";
}
