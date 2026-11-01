// Caption buttons: their geometry for the native overlay and the glyph source.

/** A rectangle in CSS pixels, measured from the window's top-right corner. */
export interface CaptionRect {
  right: number;
  top: number;
  width: number;
  height: number;
}

/** The overlay rectangle for a button, given the window's inner width. */
export function captionRect(
  button: Pick<DOMRect, "left" | "top" | "width" | "height">,
  windowWidth: number,
): CaptionRect {
  return {
    right: windowWidth - (button.left + button.width),
    top: button.top,
    width: button.width,
    height: button.height,
  };
}

export function sameRect(a: CaptionRect | undefined, b: CaptionRect): boolean {
  return (
    a !== undefined &&
    a.right === b.right &&
    a.top === b.top &&
    a.width === b.width &&
    a.height === b.height
  );
}

/** Segoe Fluent Icons code points (Segoe MDL2 Assets has the same ones). */
export const GLYPHS = {
  minimize: "",
  maximize: "",
  restore: "",
  close: "",
} as const;

export type GlyphSource = "font" | "svg";

/**
 * Whether a caption icon font is installed (Windows 11 has Segoe Fluent
 * Icons, Windows 10 Segoe MDL2 Assets). Elsewhere the buttons use inline SVG.
 * A font is present when its glyph renders at a different width than the
 * fallback's.
 */
export function detectGlyphSource(): GlyphSource {
  const context = document.createElement("canvas").getContext("2d");
  if (!context) return "svg";
  const width = (family: string) => {
    context.font = `40px ${family}`;
    return context.measureText(GLYPHS.close + GLYPHS.maximize).width;
  };
  const fallback = width("monospace");
  const present = ['"Segoe Fluent Icons"', '"Segoe MDL2 Assets"'].some(
    (family) => width(`${family}, monospace`) !== fallback,
  );
  return present ? "font" : "svg";
}
