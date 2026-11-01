import { describe, expect, it } from "vitest";
import catalog from "./en.json";
import { createTranslator, t } from "./index";

describe("t()", () => {
  it("looks up a plain message", () => {
    expect(t("titlebar.add")).toBe("Add download");
  });

  it("fills named placeholders", () => {
    expect(t("empty.shortcut", { shortcut: "Ctrl+V" })).toBe("Press Ctrl+V to paste a link");
  });

  it("selects the plural form from count and formats the number", () => {
    expect(t("view.count", { count: 0 })).toBe("0 downloads");
    expect(t("view.count", { count: 1 })).toBe("1 download");
    expect(t("view.count", { count: 1234 })).toBe("1,234 downloads");
  });

  it("falls back to `other` when a plural category is missing", () => {
    const tt = createTranslator({ "view.count": { other: "{count} items" } });
    expect(tt("view.count", { count: 1 })).toBe("1 items");
  });

  it("keeps an unfilled placeholder visible instead of dropping it", () => {
    const tt = createTranslator({ "empty.shortcut": "Press {shortcut}" });
    // @ts-expect-error: the placeholder is required by the generated types.
    expect(tt("empty.shortcut", {})).toBe("Press {shortcut}");
  });

  it("returns the key for a message missing from the catalog", () => {
    const tt = createTranslator({});
    expect(tt("titlebar.add")).toBe("titlebar.add");
  });

  it("rejects wrong arguments at compile time", () => {
    // @ts-expect-error: an unknown key.
    expect(t("no.such.key")).toBe("no.such.key");
    // @ts-expect-error: a message without placeholders takes no parameters.
    expect(t("titlebar.add", { count: 1 })).toBe("Add download");
  });
});

describe("the catalog", () => {
  const entries = Object.entries(catalog as Record<string, string | Record<string, string>>);

  it("uses sentence case and no exclamation marks", () => {
    for (const [key, value] of entries) {
      for (const text of typeof value === "string" ? [value] : Object.values(value)) {
        expect(text, key).not.toMatch(/!/);
        expect(text.trim(), key).toBe(text);
      }
    }
  });

  it("gives every plural message an `other` form that uses {count}", () => {
    for (const [key, value] of entries) {
      if (typeof value === "string") continue;
      expect(value.other, key).toContain("{count}");
    }
  });
});
