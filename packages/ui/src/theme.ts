// Theme resolution (DESIGN-PRINCIPLES.md, section 4). The setting is System,
// Light or Dark; System follows the OS, and light wins when the OS gives no
// preference. CSS does the work through color-scheme and light-dark(): this
// module only decides the attribute and tells native code which colours to use.
import resolvedColors from "./tokens/theme.json";

export type ThemeSetting = "system" | "light" | "dark";
export type Theme = "light" | "dark";
export type ThemeColorName = keyof typeof resolvedColors.light;

const DARK_QUERY = "(prefers-color-scheme: dark)";

/** The theme a setting resolves to, given whether the OS prefers dark. */
export function resolveTheme(setting: ThemeSetting, systemPrefersDark: boolean): Theme {
  if (setting === "light" || setting === "dark") return setting;
  return systemPrefersDark ? "dark" : "light";
}

/** Applies a setting to the document: `data-theme` is set only when explicit. */
export function applyTheme(root: HTMLElement, setting: ThemeSetting): void {
  if (setting === "system") root.removeAttribute("data-theme");
  else root.dataset.theme = setting;
}

/** Whether the OS currently prefers dark (false outside a browser). */
export function systemPrefersDark(): boolean {
  return typeof matchMedia === "function" && matchMedia(DARK_QUERY).matches;
}

/** Calls `onChange` whenever the OS theme changes; returns the unsubscribe. */
export function watchSystemTheme(onChange: (prefersDark: boolean) => void): () => void {
  if (typeof matchMedia !== "function") return () => {};
  const query = matchMedia(DARK_QUERY);
  const listener = (event: MediaQueryListEvent) => onChange(event.matches);
  query.addEventListener("change", listener);
  return () => query.removeEventListener("change", listener);
}

/** A semantic colour as #RRGGBB or #RRGGBBAA, resolved from the brand tokens. */
export function themeColor(theme: Theme, name: ThemeColorName): string {
  return resolvedColors[theme][name];
}

/** Parses #RRGGBB or #RRGGBBAA into [r, g, b, a] bytes. */
export function hexToRgba(hex: string): [number, number, number, number] {
  const match = /^#([0-9a-f]{6})([0-9a-f]{2})?$/i.exec(hex);
  const [, rgb, alpha] = match ?? [];
  if (rgb === undefined) throw new Error(`not a hex colour: ${hex}`);
  const n = Number.parseInt(rgb, 16);
  const a = alpha === undefined ? 255 : Number.parseInt(alpha, 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff, a];
}
