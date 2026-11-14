// Captures the real extension UI for the store images: the release build
// (`wxt build`, the code that goes into the store package), loaded unpacked
// into Playwright's Chromium against the end-to-end stub host
// (e2e/stub-host.mjs), which answers like a running Boltget 0.3.0.
//
// Two changes to a copy of the build, both outside the UI code: the
// development key, so the copy has a known ID the stub host can allow, and
// site access as a granted permission, the state after the user has allowed
// it (a script cannot answer the browser's permission prompt).
//
// The stub is registered under the release host name for Chromium and Chrome
// for Testing only, and only where no registration exists, so a real Boltget
// install is never touched; it is removed again afterwards.
import { execFileSync } from "node:child_process";
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "@playwright/test";
import extensions from "../../../../packages/native-messaging/extensions.json" with { type: "json" };
import { chromiumPath } from "./lib/browser.mjs";
import { CACHE, CAPTURES, EXTENSION_DIR, SCALE, run, savePng } from "./lib/paths.mjs";

const HOST = extensions.release.host;
const DEV_KEY = extensions.development.chromiumKey;
const ID = extensions.development.chromium[0];
const VENDORS = ["Chromium", "Google\\Chrome for Testing"];

/** What the stub host answers: a running app with its default rules and two site lists. */
const STUB = {
  app: "Boltget 0.3.0",
  capture: "accept",
  rules: {
    enabled: true,
    min_size: 1024 * 1024,
    extensions: (
      "zip rar 7z tar gz bz2 xz zst iso img exe msi msix appx dmg pkg deb rpm apk " +
      "mp4 mkv webm mov avi m4v mp3 flac wav m4a ogg opus bin dat"
    ).split(" "),
    never_sites: ["192.168.1.1", "nas.home.arpa"],
    always_sites: ["releases.ubuntu.com", "download.blender.org"],
    private_windows: false,
  },
};

function registerHost(folder, profile) {
  const manifest = JSON.stringify({
    name: HOST,
    description: "Boltget store screenshots: the end-to-end stub host",
    path: join(
      EXTENSION_DIR,
      "e2e",
      process.platform === "win32" ? "stub-host.cmd" : "stub-host.sh",
    ),
    type: "stdio",
    allowed_origins: [`chrome-extension://${ID}/`],
  });
  const file = join(folder, `${HOST}.json`);
  mkdirSync(folder, { recursive: true });
  writeFileSync(file, manifest);
  if (process.platform !== "win32") {
    // Chromium also reads the profile's own folder on Linux and macOS.
    const own = join(profile, "NativeMessagingHosts");
    mkdirSync(own, { recursive: true });
    writeFileSync(join(own, `${HOST}.json`), manifest);
    return () => rmSync(join(own, `${HOST}.json`), { force: true });
  }
  const added = [];
  for (const vendor of VENDORS) {
    const key = `HKCU\\Software\\${vendor}\\NativeMessagingHosts\\${HOST}`;
    let exists = true;
    try {
      execFileSync("reg", ["query", key], { stdio: "ignore" });
    } catch {
      exists = false;
    }
    if (exists) {
      console.warn(`  ${key} exists (a Boltget install?): left alone`);
      continue;
    }
    execFileSync("reg", ["add", key, "/ve", "/t", "REG_SZ", "/d", file, "/f"], {
      stdio: "ignore",
    });
    added.push(key);
  }
  return () => {
    for (const key of added) {
      try {
        execFileSync("reg", ["delete", key, "/f"], { stdio: "ignore" });
      } catch {
        // Already gone.
      }
    }
  };
}

/** Opens one of the extension's pages and waits until the app has answered. */
async function open(context, path, scheme, width) {
  const page = await context.newPage();
  await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
  await page.setViewportSize({ width, height: 900 });
  await page.goto(`chrome-extension://${ID}/${path}`);
  await page.getByRole("status").filter({ hasText: "Connected to" }).first().waitFor();
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(500);
  return page;
}

const slug = (text) =>
  (text ?? "")
    .toLowerCase()
    .replace(/[^a-z]+/g, "-")
    .replace(/^-|-$/g, "");

/** Builds the release extension and writes the popup and options captures. */
export async function captureExtension({ build = true } = {}) {
  if (build) run("pnpm", ["exec", "wxt", "build"], EXTENSION_DIR);

  const work = join(CACHE, "extension");
  rmSync(work, { recursive: true, force: true, maxRetries: 5 });
  const unpacked = join(work, "chrome-mv3");
  cpSync(join(EXTENSION_DIR, ".output", "chrome-mv3"), unpacked, { recursive: true });
  const manifestPath = join(unpacked, "manifest.json");
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  if ("key" in manifest) throw new Error("the release build carries a key");
  manifest.key = DEV_KEY;
  manifest.host_permissions = manifest.optional_host_permissions;
  delete manifest.optional_host_permissions;
  writeFileSync(manifestPath, JSON.stringify(manifest));

  const stubDir = join(work, "stub");
  mkdirSync(stubDir, { recursive: true });
  writeFileSync(join(stubDir, "mode.json"), JSON.stringify(STUB));
  const profile = join(work, "profile");
  const unregister = registerHost(join(work, "host"), profile);

  let context;
  try {
    context = await chromium.launchPersistentContext(profile, {
      headless: false,
      executablePath: chromiumPath(),
      deviceScaleFactor: SCALE,
      locale: "en-US",
      args: [
        "--headless=new",
        `--disable-extensions-except=${unpacked}`,
        `--load-extension=${unpacked}`,
        "--lang=en-US",
        "--hide-scrollbars",
      ],
      env: { ...process.env, BOLTGET_STUB_DIR: stubDir },
    });
    if (!context.serviceWorkers().some((w) => w.url().includes(ID))) {
      await context.waitForEvent("serviceworker", { predicate: (w) => w.url().includes(ID) });
    }
    await new Promise((r) => setTimeout(r, 1000));
    // The install opens the welcome page; the captures open their own pages.
    for (const page of context.pages()) if (page.url().includes(ID)) await page.close();

    for (const scheme of ["light", "dark"]) {
      const popup = await open(context, "popup.html", scheme, 320);
      await savePng(
        await popup.locator("main").screenshot(),
        join(CAPTURES, `popup-${scheme}.png`),
      );
      await popup.close();

      const options = await open(context, "options.html", scheme, 760);
      await options.getByRole("heading", { name: "What goes to Boltget" }).waitFor();
      await savePng(
        await options.screenshot({ fullPage: true }),
        join(CAPTURES, `options-${scheme}.png`),
      );
      const sections = options.locator("main > section");
      const count = await sections.count();
      for (let i = 0; i < count; i += 1) {
        const name = slug(await sections.nth(i).locator("h2").first().textContent());
        await savePng(
          await sections.nth(i).screenshot(),
          join(CAPTURES, `options-${name}-${scheme}.png`),
        );
      }
      await options.close();
    }
  } finally {
    await context?.close();
    unregister();
  }
}
