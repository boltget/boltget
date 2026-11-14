// What the extension keeps in `storage.local`: the capture rules (the app's,
// or the user's edits), the popup's switch, a pause, and the modifier-key
// setting. Nothing about downloads is ever stored.
import type { CaptureRules } from "@boltget/native-messaging";
import type { CaptureSettings } from "./capture";
import { DEFAULT_RULES, normaliseRules } from "./rules";

/** Everything stored. */
export interface Settings {
  /** The rules in force. */
  rules: CaptureRules;
  /**
   * The user edited the rules in options: keep them instead of the app's
   * (until the app can edit them itself, BROWSER-INTEGRATION.md, "Capture
   * rules").
   */
  rulesEdited: boolean;
  /** The last rules the app sent, for "Use Boltget's rules". */
  appRules: CaptureRules | null;
  /** The popup's capture switch. */
  captureOn: boolean;
  /** Capture is paused until this time (ms since 1970); 0 when not. */
  pausedUntil: number;
  /** Alt bypasses and Alt+Shift forces capture on link clicks. */
  modifierKeys: boolean;
}

/** How long "Pause for 10 minutes" pauses. */
export const PAUSE_MS = 10 * 60 * 1000;

export const DEFAULT_SETTINGS: Settings = {
  rules: DEFAULT_RULES,
  rulesEdited: false,
  appRules: null,
  captureOn: true,
  pausedUntil: 0,
  modifierKeys: true,
};

const KEYS = Object.keys(DEFAULT_SETTINGS) as (keyof Settings)[];

/** The storage area, injected for tests (`browser.storage.local` in the extension). */
export interface Store {
  get(keys: string[]): Promise<Record<string, unknown>>;
  set(items: Record<string, unknown>): Promise<void>;
}

/** The stored settings, with defaults for what is missing or broken. */
export function parseSettings(raw: Record<string, unknown>): Settings {
  const bool = (key: keyof Settings, fallback: boolean) =>
    typeof raw[key] === "boolean" ? (raw[key] as boolean) : fallback;
  const paused = raw.pausedUntil;
  return {
    rules: raw.rules === undefined ? DEFAULT_RULES : normaliseRules(raw.rules),
    rulesEdited: bool("rulesEdited", false),
    appRules: raw.appRules ? normaliseRules(raw.appRules) : null,
    captureOn: bool("captureOn", true),
    pausedUntil: typeof paused === "number" && Number.isFinite(paused) ? paused : 0,
    modifierKeys: bool("modifierKeys", true),
  };
}

export async function loadSettings(store: Store): Promise<Settings> {
  return parseSettings(await store.get(KEYS));
}

/** Stores a change to some settings. */
export async function saveSettings(store: Store, change: Partial<Settings>): Promise<void> {
  await store.set(change);
}

/**
 * The app answered `hello` with its rules: remember them, and use them unless
 * the user has edited the rules here.
 */
export async function adoptAppRules(store: Store, rules: CaptureRules): Promise<void> {
  const clean = normaliseRules(rules);
  const current = await loadSettings(store);
  await store.set(current.rulesEdited ? { appRules: clean } : { appRules: clean, rules: clean });
}

/** The user's edits from options. */
export async function editRules(store: Store, rules: CaptureRules): Promise<void> {
  await store.set({ rules: normaliseRules(rules), rulesEdited: true });
}

/** "Use Boltget's rules": back to the app's (or the defaults). */
export async function resetRules(store: Store): Promise<CaptureRules> {
  const current = await loadSettings(store);
  const rules = current.appRules ?? DEFAULT_RULES;
  await store.set({ rules, rulesEdited: false });
  return rules;
}

/** Whether capture is paused at `now`. */
export function isPaused(settings: Settings, now: number): boolean {
  return settings.pausedUntil > now;
}

/** The settings `shouldCapture` applies at `now`. */
export function captureSettings(settings: Settings, now: number): CaptureSettings {
  return { ...settings.rules, active: settings.captureOn && !isPaused(settings, now) };
}
