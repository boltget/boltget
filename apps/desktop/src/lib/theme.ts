// The app's theme at runtime. The setting is System for now (there is no
// settings window yet); CSS follows it through color-scheme, and native code
// is told the resolved theme so the window background and border match.
import {
  applyTheme,
  resolveTheme,
  systemPrefersDark,
  watchSystemTheme,
  type Theme,
  type ThemeSetting,
} from "@boltget/ui/theme";
import { setWindowChrome } from "./native";

let setting: ThemeSetting = "system";
let systemDark = systemPrefersDark();

export function currentTheme(): Theme {
  return resolveTheme(setting, systemDark);
}

async function sync(): Promise<void> {
  applyTheme(document.documentElement, setting);
  await setWindowChrome(currentTheme());
}

/** Applies the theme and keeps it in sync with the OS. */
export async function startTheme(initial: ThemeSetting = "system"): Promise<void> {
  setting = initial;
  watchSystemTheme((dark) => {
    systemDark = dark;
    void sync();
  });
  await sync();
}

/**
 * Review aid for debug builds only: Ctrl+Shift+L flips between light and
 * dark, overriding System until the app restarts.
 */
export function enableThemeToggle(): void {
  addEventListener("keydown", (event) => {
    if (!(event.ctrlKey && event.shiftKey && event.code === "KeyL")) return;
    event.preventDefault();
    setting = currentTheme() === "dark" ? "light" : "dark";
    void sync();
  });
}

/** Reads a theme forced for review (BOLTGET_DEV_THEME in debug builds). */
export function parseThemeSetting(value: string | null): ThemeSetting {
  return value === "light" || value === "dark" ? value : "system";
}
