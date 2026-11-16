import { describe, expect, it } from "vitest";
import { DEFAULT_RULES } from "./rules";
import {
  adoptAppRules,
  captureSettings,
  DEFAULT_SETTINGS,
  editRules,
  loadSettings,
  parseSettings,
  resetRules,
  type Store,
} from "./settings";

function memory(): Store & { data: Record<string, unknown> } {
  const data: Record<string, unknown> = {};
  return {
    data,
    async get(keys) {
      return Object.fromEntries(keys.filter((k) => k in data).map((k) => [k, data[k]]));
    },
    async set(items) {
      Object.assign(data, structuredClone(items));
    },
  };
}

const appRules = { ...DEFAULT_RULES, min_size: 5 << 20, never_sites: ["bank.example"] };

describe("settings", () => {
  it("start from the defaults, and survive broken storage", async () => {
    expect(await loadSettings(memory())).toEqual(DEFAULT_SETTINGS);
    expect(
      parseSettings({ rules: 7, captureOn: "yes", pausedUntil: "soon", modifierKeys: null }),
    ).toEqual({ ...DEFAULT_SETTINGS, rules: DEFAULT_RULES });
  });

  it("take the app's rules, and keep edits made here while the app's stay the same", async () => {
    const store = memory();
    await adoptAppRules(store, appRules);
    expect((await loadSettings(store)).rules).toEqual(appRules);

    const mine = { ...appRules, min_size: 0 };
    await editRules(store, mine);
    // The same rules again (every hello): the edits stay.
    await adoptAppRules(store, { ...appRules });
    const s = await loadSettings(store);
    expect(s.rules).toEqual(mine);
    expect(s.rulesEdited).toBe(true);
    expect(s.appRules).toEqual(appRules);
  });

  it("let a change made in the app replace edits made here: the latest change wins", async () => {
    const store = memory();
    await adoptAppRules(store, appRules);
    await editRules(store, { ...appRules, min_size: 0 });
    const changed = { ...appRules, min_size: 9, always_sites: ["cdn.example"] };
    await adoptAppRules(store, changed);
    const s = await loadSettings(store);
    expect(s.rules).toEqual(changed);
    expect(s.rulesEdited).toBe(false);
    expect(s.appRules).toEqual(changed);
  });

  it("keep edits made before the app was ever reached", async () => {
    const store = memory();
    const mine = { ...DEFAULT_RULES, enabled: false };
    await editRules(store, mine);
    await adoptAppRules(store, appRules);
    const s = await loadSettings(store);
    expect(s.rules).toEqual(mine);
    expect(s.rulesEdited).toBe(true);
    expect(s.appRules).toEqual(appRules);
  });

  it("compare the app's rules as normalised", async () => {
    const store = memory();
    await adoptAppRules(store, appRules);
    await editRules(store, { ...appRules, enabled: false });
    // The same rules, written differently.
    await adoptAppRules(store, { ...appRules, never_sites: ["Bank.example", "bank.example"] });
    expect((await loadSettings(store)).rulesEdited).toBe(true);
  });

  it("reset goes back to the app's last rules, or the defaults", async () => {
    const store = memory();
    await editRules(store, { ...DEFAULT_RULES, enabled: false });
    expect(await resetRules(store)).toEqual(DEFAULT_RULES);
    await adoptAppRules(store, appRules);
    await editRules(store, { ...DEFAULT_RULES, enabled: false });
    expect(await resetRules(store)).toEqual(appRules);
    expect((await loadSettings(store)).rulesEdited).toBe(false);
  });

  it("the switch and the pause make capture inactive", () => {
    const now = 1_000_000;
    expect(captureSettings(DEFAULT_SETTINGS, now).active).toBe(true);
    expect(captureSettings({ ...DEFAULT_SETTINGS, captureOn: false }, now).active).toBe(false);
    expect(captureSettings({ ...DEFAULT_SETTINGS, pausedUntil: now + 1 }, now).active).toBe(false);
    expect(captureSettings({ ...DEFAULT_SETTINGS, pausedUntil: now }, now).active).toBe(true);
  });
});
