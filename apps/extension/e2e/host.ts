// Registers the stub native host (stub-host.mjs) for the browsers the tests
// launch, under its own name (app.boltget.native_host_e2e), so a test never
// touches a real or development Boltget registration.
//
// - Windows: a manifest in a temporary folder and an HKCU key per browser
//   vendor (`reg add`), removed again by `unregister`.
// - Linux: the manifest in each browser's per-user NativeMessagingHosts
//   folder, and in the test profile's own, which Chromium also reads.
import { execFileSync } from "node:child_process";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const E2E_HOST_NAME = "app.boltget.native_host_e2e";

const here = fileURLToPath(new URL(".", import.meta.url));

/** Registry paths (under HKCU\Software) of the browsers the tests may run. */
const WINDOWS_VENDORS = [
  "Chromium",
  "Google\\Chrome for Testing",
  "Google\\Chrome",
  "Microsoft\\Edge",
];

/** Per-user manifest folders on Linux, besides the profile's own. */
const LINUX_DIRS = [
  ".config/chromium",
  ".config/google-chrome-for-testing",
  ".config/google-chrome",
  ".config/microsoft-edge",
].map((d) => join(homedir(), d, "NativeMessagingHosts"));

/** A registration to undo. */
export interface Registration {
  unregister(): void;
}

/** Points the e2e host name at the stub for `extensionId`. */
export function registerStubHost(
  extensionId: string,
  folder: string,
  profile: string,
): Registration {
  const windows = process.platform === "win32";
  const manifest = {
    name: E2E_HOST_NAME,
    description: "Boltget end-to-end test stub",
    path: join(here, windows ? "stub-host.cmd" : "stub-host.sh"),
    type: "stdio",
    allowed_origins: [`chrome-extension://${extensionId}/`],
  };
  const json = JSON.stringify(manifest, null, 2);
  const file = `${E2E_HOST_NAME}.json`;

  if (windows) {
    mkdirSync(folder, { recursive: true });
    const path = join(folder, file);
    writeFileSync(path, json);
    const keys = WINDOWS_VENDORS.map(
      (vendor) => `HKCU\\Software\\${vendor}\\NativeMessagingHosts\\${E2E_HOST_NAME}`,
    );
    for (const key of keys) {
      execFileSync("reg", ["add", key, "/ve", "/t", "REG_SZ", "/d", path, "/f"], {
        stdio: "ignore",
      });
    }
    return {
      unregister() {
        for (const key of keys) {
          try {
            execFileSync("reg", ["delete", key, "/f"], { stdio: "ignore" });
          } catch {
            // Already gone.
          }
        }
        rmSync(path, { force: true });
      },
    };
  }

  const dirs = [join(profile, "NativeMessagingHosts"), ...LINUX_DIRS];
  for (const dir of dirs) {
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, file), json);
  }
  return {
    unregister() {
      for (const dir of dirs) rmSync(join(dir, file), { force: true });
    },
  };
}
