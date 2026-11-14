import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import catalog from "./en.json";
import { t } from "./i18n";

const root = new URL("..", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sources(path);
    return /\.(ts|svelte)$/.test(name) && !name.endsWith(".test.ts") ? [path] : [];
  });
}

describe("catalog", () => {
  it("fills placeholders", () => {
    expect(t("status.connected", { app: "Boltget 0.3.0" })).toBe("Connected to Boltget 0.3.0");
    expect(t("status.connected")).toBe("Connected to {app}");
  });

  it("has no unused messages", () => {
    const code = [...sources(join(root, "entrypoints")), ...sources(join(root, "lib"))]
      .map((p) => readFileSync(p, "utf8"))
      .join("\n");
    const unused = Object.keys(catalog).filter((key) => !code.includes(`"${key}"`));
    expect(unused).toEqual([]);
  });

  it("the manifest's strings exist for every flavor", () => {
    const locales = JSON.parse(
      readFileSync(join(root, "public/_locales/en/messages.json"), "utf8"),
    ) as Record<string, { message: string }>;
    const config = readFileSync(join(root, "wxt.config.ts"), "utf8");
    for (const [, key] of config.matchAll(/__MSG_(\w+)__/g)) {
      expect(locales[key ?? ""]?.message, key).toBeTruthy();
    }
  });
});
