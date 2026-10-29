// The site's string catalog (docs/DESIGN-PRINCIPLES.md, Copy and strings).
// Every user-facing string comes from en.json by key. Messages are whole
// sentences with named placeholders, never concatenated fragments.
import en from "./en.json";

type Catalog = typeof en;

/** Dotted paths to every string leaf of the catalog, e.g. `"home.headline"`. */
type Leaves<T, Prefix extends string = ""> = {
  [K in keyof T & string]: T[K] extends string
    ? `${Prefix}${K}`
    : Leaves<T[K], `${Prefix}${K}.`>;
}[keyof T & string];

/** Paths to plural messages (objects with `one` and `other`). */
type Plurals<T, Prefix extends string = ""> = {
  [K in keyof T & string]: T[K] extends { one: string; other: string }
    ? `${Prefix}${K}`
    : T[K] extends string
      ? never
      : Plurals<T[K], `${Prefix}${K}.`>;
}[keyof T & string];

export type MessageKey = Leaves<Catalog>;
export type PluralKey = Plurals<Catalog>;
export type Params = Record<string, string | number>;

const plural = new Intl.PluralRules("en");

function lookup(key: string): unknown {
  let node: unknown = en;
  for (const part of key.split(".")) {
    if (node === null || typeof node !== "object" || !(part in node)) {
      throw new Error(`Missing string: ${key}`);
    }
    node = (node as Record<string, unknown>)[part];
  }
  return node;
}

/** Replaces `{name}` placeholders. A missing parameter fails the build. */
export function format(message: string, params: Params = {}): string {
  return message.replace(/\{(\w+)\}/g, (_, name: string) => {
    const value = params[name];
    if (value === undefined) throw new Error(`Missing parameter {${name}} in "${message}"`);
    return String(value);
  });
}

/** The message for `key`, with placeholders filled in. */
export function t(key: MessageKey, params?: Params): string {
  const message = lookup(key);
  if (typeof message !== "string") throw new Error(`Not a string: ${key}`);
  return format(message, params);
}

/** A message that is looked up at runtime (for example from data). */
export function tDynamic(key: string, params?: Params): string {
  return t(key as MessageKey, params);
}

/** The plural form of `key` for `count`, with `{count}` and other placeholders. */
export function tp(key: PluralKey, count: number, params: Params = {}): string {
  const forms = lookup(key) as Record<string, string>;
  const form = forms[plural.select(count)] ?? forms.other;
  if (form === undefined) throw new Error(`No plural form for ${key}`);
  return format(form, { count, ...params });
}

/** Raw plural forms, for islands that pluralize in the browser. */
export function forms(key: PluralKey): { one: string; other: string } {
  return lookup(key) as { one: string; other: string };
}

/** The message for `key` with its placeholders left in, for islands that fill them in the browser. */
export function raw(key: MessageKey): string {
  const message = lookup(key);
  if (typeof message !== "string") throw new Error(`Not a string: ${key}`);
  return message;
}
