// The page's side of the native shell (src-tauri/src/commands.rs), through
// the generated bindings. Every call degrades to a no-op outside Tauri, so
// the UI also runs in a plain browser (`pnpm --filter @boltget/desktop dev`)
// for design review.
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { hexToRgba, themeColor, type Theme } from "@boltget/ui/theme";
import { commands, events, type AppInfo, type CaptionMaximize } from "./bindings";
import type { CaptionRect } from "./caption";

export const inTauri = isTauri();

export type { AppInfo };

export async function appInfo(): Promise<AppInfo> {
  if (!inTauri) {
    // The UI alone in a browser: `?theme=light|dark` stands in for
    // BOLTGET_DEV_THEME, for screenshots of both themes.
    const devTheme = import.meta.env.DEV ? new URLSearchParams(location.search).get("theme") : null;
    return { version: "0.0.0", debug: import.meta.env.DEV, devTheme };
  }
  return commands.appInfo();
}

/** Shows the hidden main window; called once the first frame is ready. */
export async function showMainWindow(): Promise<void> {
  if (inTauri) await commands.showMainWindow();
}

function rgb(hex: string): [number, number, number] {
  const [r, g, b] = hexToRgba(hex);
  return [r, g, b];
}

/** Tells native code the page's theme: window background and border. */
export async function setWindowChrome(theme: Theme): Promise<void> {
  if (!inTauri) return;
  await commands.setWindowChrome(
    theme,
    rgb(themeColor(theme, "content")),
    rgb(themeColor(theme, "window-border")),
  );
}

/** Where the maximize button is, for the native snap layout overlay. */
export async function setCaptionLayout(maximize: CaptionRect): Promise<void> {
  if (inTauri) await commands.setCaptionLayout(maximize);
}

export type MaximizeButtonState = CaptionMaximize;

/** Hover and press of the maximize button, reported by the native overlay. */
export async function onMaximizeButton(
  handler: (state: MaximizeButtonState) => void,
): Promise<() => void> {
  if (!inTauri) return () => {};
  return events.captionMaximize.listen((event) => handler(event.payload));
}

export const appWindow = {
  minimize: () => (inTauri ? getCurrentWindow().minimize() : Promise.resolve()),
  toggleMaximize: () => (inTauri ? getCurrentWindow().toggleMaximize() : Promise.resolve()),
  close: () => (inTauri ? getCurrentWindow().close() : Promise.resolve()),
  isMaximized: () => (inTauri ? getCurrentWindow().isMaximized() : Promise.resolve(false)),
  /** Calls back with the maximized state now and after every resize. */
  async onMaximizedChange(handler: (maximized: boolean) => void): Promise<() => void> {
    if (!inTauri) return () => {};
    const window = getCurrentWindow();
    handler(await window.isMaximized());
    return window.onResized(async () => handler(await window.isMaximized()));
  },
  /** Calls back when the window gains or loses focus. */
  async onFocusChange(handler: (focused: boolean) => void): Promise<() => void> {
    if (!inTauri) {
      const onFocus = () => handler(true);
      const onBlur = () => handler(false);
      addEventListener("focus", onFocus);
      addEventListener("blur", onBlur);
      return () => {
        removeEventListener("focus", onFocus);
        removeEventListener("blur", onBlur);
      };
    }
    return getCurrentWindow().onFocusChanged((event) => handler(event.payload));
  },
};
