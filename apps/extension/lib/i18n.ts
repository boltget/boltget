// The extension's string catalog (TECH-STACK.md, ADR-015, as in the app):
// every user-facing string lives in en.json and is looked up by key. The
// manifest's own strings (name, description) are in public/_locales, where the
// browsers and stores read them.
import catalog from "./en.json";

/** A message key. */
export type MessageKey = keyof typeof catalog;

const PLACEHOLDER = /\{(\w+)\}/g;

/** Looks up a message and fills in its `{placeholders}`. */
export function t(key: MessageKey, params: Record<string, string | number> = {}): string {
  const text: string = catalog[key] ?? key;
  return text.replace(PLACEHOLDER, (whole, name: string) => {
    const value = params[name];
    return value === undefined ? whole : String(value);
  });
}
