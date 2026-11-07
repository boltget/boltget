// Contrast of the app's colour pairs in both themes, computed from the
// generated tokens (WCAG 2.2: 4.5:1 for text, 3:1 for the boundaries and
// glyphs that identify a control or a state). docs/ACCESSIBILITY.md lists the
// pairs and their ratios; this test keeps them from regressing.
import { describe, expect, it } from "vitest";
import tokens from "./tokens/theme.json";

type Theme = keyof typeof tokens;
type Token = keyof (typeof tokens)["light"];
type Rgba = [number, number, number, number];

function rgba(hex: string): Rgba {
  const h = hex.slice(1);
  const ch = (i: number) => parseInt(h.slice(i, i + 2), 16);
  return [ch(0), ch(2), ch(4), h.length > 6 ? ch(6) / 255 : 1];
}

/** `fg` composited over an opaque `bg`. */
function over(fg: Rgba, bg: Rgba): Rgba {
  const a = fg[3];
  return [0, 1, 2]
    .map((i) => (fg[i] as number) * a + (bg[i] as number) * (1 - a))
    .concat(1) as Rgba;
}

function luminance([r, g, b]: Rgba): number {
  const lin = (v: number) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function contrast(a: Rgba, b: Rgba): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (hi + 0.05) / (lo + 0.05);
}

/** A surface: a token, or a translucent token over one ("overlay-hover/chrome"). */
function surface(theme: Theme, name: string): Rgba {
  const [top, under] = name.split("/") as [string, string | undefined];
  const colour = rgba(tokens[theme][top as Token]);
  if (colour[3] === 1) return colour;
  return over(colour, surface(theme, under ?? "content"));
}

const SURFACES = [
  "content",
  "chrome",
  "raised",
  "overlay-hover/content",
  "overlay-hover/chrome",
  "overlay-hover/raised",
  "accent-tint/content",
];

/** [foreground, surfaces, minimum ratio] */
const PAIRS: [Token, string[], number][] = [
  ["text", SURFACES, 4.5],
  ["text-muted", SURFACES, 4.5],
  ["text-subtle", SURFACES, 4.5],
  ["accent", ["content", "chrome", "raised"], 4.5],
  ["error", ["content", "raised"], 4.5],
  ["warning", ["content", "chrome", "raised"], 4.5],
  ["info", ["content", "chrome", "raised"], 4.5],
  ["on-accent", ["accent", "accent-hover"], 4.5],
  ["on-danger", ["danger"], 4.5],
  ["on-caption-close", ["caption-close"], 4.5],
  ["on-tooltip", ["tooltip"], 4.5],
  ["focus", ["content", "chrome", "raised"], 3],
  ["control-stroke", ["content", "chrome", "raised"], 3],
  ["progress-idle", ["content", "progress-track/content"], 3],
  ["success", ["content", "chrome", "raised"], 3],
];

describe.each(["light", "dark"] as Theme[])("%s theme", (theme) => {
  for (const [fg, surfaces, min] of PAIRS) {
    it.each(surfaces)(`${fg} on %s is at least ${min}:1`, (bg) => {
      const back = surface(theme, bg);
      const front = over(rgba(tokens[theme][fg]), back);
      expect(contrast(front, back)).toBeGreaterThanOrEqual(min);
    });
  }
});
