import { describe, expect, it } from "vitest";
import { isLinkClick, ModifierLog, modifiersOf } from "./modifiers";

describe("modifiers", () => {
  it("Alt bypasses, Alt+Shift forces, Shift alone does nothing", () => {
    expect(modifiersOf(false, false)).toBe("none");
    expect(modifiersOf(false, true)).toBe("none");
    expect(modifiersOf(true, false)).toBe("bypass");
    expect(modifiersOf(true, true)).toBe("force");
  });

  it("checks what the content script posts", () => {
    const click = { type: "boltget:click", href: "https://e.com", alt: true, shift: false };
    expect(isLinkClick(click)).toBe(true);
    expect(isLinkClick({ ...click, href: 1 })).toBe(false);
    expect(isLinkClick({ ...click, type: "other" })).toBe(false);
    expect(isLinkClick({ ...click, href: "x".repeat(40_000) })).toBe(false);
    expect(isLinkClick(null)).toBe(false);
  });

  it("applies a click's keys to a download of that link within 2 s, once", () => {
    const log = new ModifierLog();
    log.record("https://e.com/a.zip", "bypass", 0);
    expect(log.take(["https://e.com/a.zip"], 1999)).toBe("bypass");
    expect(log.take(["https://e.com/a.zip"], 1999)).toBe("none");
    log.record("https://e.com/b.zip", "force", 0);
    expect(log.take(["https://e.com/b.zip"], 2001)).toBe("none");
  });

  it("matches the link or the final URL", () => {
    const log = new ModifierLog();
    log.record("https://e.com/get", "force", 0);
    expect(log.take(["https://cdn.e.com/a.zip", "https://e.com/get"], 10)).toBe("force");
  });

  it("a click without keys clears an earlier one", () => {
    const log = new ModifierLog();
    log.record("https://e.com/a.zip", "bypass", 0);
    log.record("https://e.com/a.zip", "none", 1);
    expect(log.take(["https://e.com/a.zip"], 2)).toBe("none");
  });
});
