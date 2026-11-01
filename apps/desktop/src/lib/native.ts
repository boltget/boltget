// The page's side of the native shell (src-tauri/src/commands.rs). Every call
// degrades to a no-op outside Tauri, so the UI also runs in a plain browser
// (`pnpm --filter @boltget/desktop dev`) for design review.
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { hexToRgba, themeColor, type Theme } from "@boltget/ui/theme";
import type { CaptionRect } from "./caption";

export const inTauri = isTauri();

export interface AppInfo {
  version: string;
  debug: boolean;
  /** Debug builds only: a theme forced with BOLTGET_DEV_THEME. */
  devTheme: string | null;
}

export async function appInfo(): Promise<AppInfo> {
  if (!inTauri) return { version: "0.0.0", debug: import.meta.env.DEV, devTheme: null };
  return invoke<AppInfo>("app_info");
}

/** Shows the hidden main window; called once the first frame is ready. */
export async function showMainWindow(): Promise<void> {
  if (inTauri) await invoke("show_main_window");
}

/** Tells native code the page's theme: window background and border. */
export async function setWindowChrome(theme: Theme): Promise<void> {
  if (!inTauri) return;
  const rgb = (hex: string) => hexToRgba(hex).slice(0, 3);
  await invoke("set_window_chrome", {
    theme,
    background: rgb(themeColor(theme, "content")),
    border: rgb(themeColor(theme, "window-border")),
  });
}

/** Where the maximize button is, for the native snap layout overlay. */
export async function setCaptionLayout(maximize: CaptionRect): Promise<void> {
  if (inTauri) await invoke("set_caption_layout", { maximize });
}

export type MaximizeButtonState = "rest" | "hover" | "press";

/** Hover and press of the maximize button, reported by the native overlay. */
export async function onMaximizeButton(
  handler: (state: MaximizeButtonState) => void,
): Promise<() => void> {
  if (!inTauri) return () => {};
  return listen<MaximizeButtonState>("caption-maximize", (event) => handler(event.payload));
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
