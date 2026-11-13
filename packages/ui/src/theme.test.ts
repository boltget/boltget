import { describe, expect, it } from "vitest";
import brand from "../../../brand/tokens/tokens.json";
import { applyTheme, hexToRgba, resolveTheme, themeColor } from "./theme";

describe("resolveTheme", () => {
  it("follows the OS for System", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
  });

  it("ignores the OS for an explicit setting", () => {
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });
});

describe("applyTheme", () => {
  // A minimal stand-in for <html>: only the dataset API is used.
  function fakeRoot() {
    const dataset: Record<string, string> = {};
    return {
      dataset,
      removeAttribute: (name: string) => {
        if (name === "data-theme") delete dataset.theme;
      },
    } as unknown as HTMLElement;
  }

  it("sets data-theme only for an explicit setting", () => {
    const root = fakeRoot();
    applyTheme(root, "dark");
    expect(root.dataset.theme).toBe("dark");
    applyTheme(root, "system");
    expect(root.dataset.theme).toBeUndefined();
    applyTheme(root, "light");
    expect(root.dataset.theme).toBe("light");
  });
});

describe("theme colours", () => {
  it("come from the brand tokens (Daylight content white, Ink content ink)", () => {
    expect(themeColor("light", "content")).toBe(brand.white);
    expect(themeColor("dark", "content")).toBe(brand.ink);
    expect(themeColor("light", "accent")).toBe(brand.theme.light.accent);
    expect(themeColor("dark", "accent")).toBe(brand.theme.dark.accent);
  });

  it("give the Daylight chrome the value in APP-DESIGN.md", () => {
    expect(themeColor("light", "chrome")).toBe("#F6F6F7");
  });

  it("parse to RGBA bytes", () => {
    expect(hexToRgba("#F6F6F7")).toEqual([246, 246, 247, 255]);
    expect(hexToRgba("#0B0B0F14")).toEqual([11, 11, 15, 20]);
    expect(() => hexToRgba("rgb(0 0 0)")).toThrow();
  });
});
