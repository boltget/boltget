// Asks the host who is there, for the popup and the options page, and takes
// the app's rules when it sends them.
import { connectionStatus, type ConnectionStatus } from "../native";
import { detectBrowser, extensionVersion, localStore, nativeClient } from "../platform";
import { adoptAppRules } from "../settings";

/** Where to get the app. */
export const DOWNLOAD_URL = "https://boltget.app/download";

export async function checkConnection(): Promise<ConnectionStatus> {
  const result = await nativeClient().request({
    type: "hello",
    extension: extensionVersion(),
    browser: detectBrowser(),
  });
  if (result.type === "hello" && result.rules) await adoptAppRules(localStore, result.rules);
  return connectionStatus(result);
}
