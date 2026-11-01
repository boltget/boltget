// The app's engine store, wired to the backend through the generated
// bindings. Outside Tauri (the UI alone in a browser) nothing connects and
// the store stays empty, in the "starting" state.
import { Channel, isTauri } from "@tauri-apps/api/core";
import { isPlainMessageKey, t } from "@boltget/ui";
import { commands, type EngineMessage, type UiError } from "../bindings";
import { DownloadStore, type EngineApi } from "./store.svelte";

export { Download, DownloadStore, type EngineApi, type Result } from "./store.svelte";

export const tauriEngineApi: EngineApi = {
  async subscribe(onMessage) {
    if (!isTauri()) return;
    const channel = new Channel<EngineMessage>();
    channel.onmessage = onMessage;
    await commands.subscribeEngine(channel);
  },
  commands,
};

/** The one store the app uses. */
export const downloads = new DownloadStore(tauriEngineApi);

/** The user-facing text of an error the backend sent. */
export function errorText(error: Pick<UiError, "key">): string {
  return isPlainMessageKey(error.key) ? t(error.key) : t("error.unknown");
}
