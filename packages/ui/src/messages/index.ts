// The string catalog (TECH-STACK.md, ADR-015). Every user-facing string lives
// in en.json and is looked up by key; components never contain literals.
//
// A message is either a string with named placeholders, "Press {shortcut}",
// or a plural message keyed by Intl plural category, selected by `count`:
// { "one": "{count} download", "other": "{count} downloads" }.
// Placeholder types come from keys.ts, generated from en.json.
import catalog from "./en.json";
import type { MessageParams } from "./keys";

export type { MessageParams } from "./keys";
export type MessageKey = keyof MessageParams;

/** Keys of messages that take no placeholders. */
export type PlainMessageKey = {
  [K in MessageKey]: [MessageParams[K]] extends [never] ? K : never;
}[MessageKey];

type Plural = Partial<Record<Intl.LDMLPluralRule, string>> & { other: string };
type Catalog = Record<string, string | Plural>;

/** Arguments after the key: nothing, or the message's placeholder values. */
export type MessageArgs<K extends MessageKey> = [MessageParams[K]] extends [never]
  ? []
  : [params: MessageParams[K]];

const LOCALE = "en";
const plurals = new Intl.PluralRules(LOCALE);
const numbers = new Intl.NumberFormat(LOCALE);
const PLACEHOLDER = /\{(\w+)\}/g;
const PLACEHOLDER_TEST = /\{\w+\}/;

/** Builds a `t()` for a catalog; the app uses the default one below. */
export function createTranslator(messages: Catalog) {
  return function t<K extends MessageKey>(key: K, ...args: MessageArgs<K>): string {
    const params: Record<string, string | number> = args[0] ?? {};
    const entry = messages[key];
    if (entry === undefined) return key;

    let text: string;
    if (typeof entry === "string") {
      text = entry;
    } else {
      const count = Number(params.count ?? 0);
      text = entry[plurals.select(count)] ?? entry.other;
    }

    return text.replace(PLACEHOLDER, (whole, name: string) => {
      const value = params[name];
      if (value === undefined) return whole;
      return typeof value === "number" ? numbers.format(value) : value;
    });
  };
}

/** Looks up a message by key and fills in its placeholders. */
export const t = createTranslator(catalog);

/**
 * Whether `key` is a message without placeholders: for keys that arrive at
 * run time, such as the error keys the backend sends.
 */
export function isPlainMessageKey(key: string): key is PlainMessageKey {
  const entry = (catalog as Catalog)[key];
  return typeof entry === "string" && !PLACEHOLDER_TEST.test(entry);
}
