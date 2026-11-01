import "@boltget/ui/styles.css";
import { mount } from "svelte";
import App from "./App.svelte";
import { commands } from "./lib/bindings";
import { downloads } from "./lib/engine";
import { markWindowShown } from "./lib/lifecycle";
import { appInfo, showMainWindow } from "./lib/native";
import { enableThemeToggle, parseThemeSetting, startTheme } from "./lib/theme";

const target = document.getElementById("app");
if (!target) throw new Error("index.html has no #app element");
mount(App, { target });

/** Resolves on the next frame, or soon anyway: a hidden window may not paint. */
function nextFrame(): Promise<void> {
  return new Promise((resolve) => {
    requestAnimationFrame(() => resolve());
    setTimeout(resolve, 50);
  });
}

function within(ms: number, work: Promise<unknown>): Promise<unknown> {
  return Promise.race([work, new Promise((resolve) => setTimeout(resolve, ms))]);
}

// The window stays hidden until the first frame is ready: theme applied,
// fonts loaded (they are bundled, so this takes milliseconds) and laid out.
// There is no splash screen (APP-DESIGN #28, #30).
async function boot(): Promise<void> {
  // The engine streams in while the window gets ready; the first snapshot
  // comes from local state, so it is there by the first frame or soon after.
  const reportVisibility = () => void downloads.setVisible(document.visibilityState === "visible");
  // A reloaded page starts visible without an event, while the backend may
  // still remember the old page as hidden (it then sends no batches): say so.
  void downloads.connect().then(reportVisibility);
  document.addEventListener("visibilitychange", reportVisibility);
  const info = await appInfo();
  await startTheme(parseThemeSetting(info.devTheme));
  await within(500, document.fonts.ready);
  await nextFrame();
  await showMainWindow();
  markWindowShown();
  if (info.debug) {
    enableThemeToggle();
    // Debug builds: the store and the generated commands, for the devtools
    // console (`__boltget.commands.addDownload({ url })`).
    Object.assign(window, { __boltget: { downloads, commands } });
  }
}

void boot();
