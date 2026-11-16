import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { NativeResult } from "./native";
import { DEFAULT_RULES } from "./rules";
import { editRules, loadSettings, type Store } from "./settings";
import { RULES_FRESH_MS, RULES_WAIT_MS, RulesSync } from "./sync";

function memory(): Store {
  const data: Record<string, unknown> = {};
  return {
    async get(keys) {
      return Object.fromEntries(keys.filter((k) => k in data).map((k) => [k, data[k]]));
    },
    async set(items) {
      Object.assign(data, structuredClone(items));
    },
  };
}

function hello(minSize: number): NativeResult {
  return {
    v: 1,
    type: "hello",
    app: "Boltget 0.3.0",
    host: "0.3.0",
    oldest: 1,
    current: 1,
    rules: { ...DEFAULT_RULES, min_size: minSize },
  };
}

describe("RulesSync", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("asks before a download only when the last answer is old", async () => {
    const store = memory();
    let size = 5;
    const ask = vi.fn(async () => hello(size));
    const sync = new RulesSync(ask, store);

    await sync.now(0);
    expect(ask).toHaveBeenCalledTimes(1);
    expect((await loadSettings(store)).rules.min_size).toBe(5);

    size = 9;
    await sync.fresh(RULES_FRESH_MS - 1);
    expect(ask).toHaveBeenCalledTimes(1);
    expect((await loadSettings(store)).rules.min_size).toBe(5);

    await sync.fresh(RULES_FRESH_MS);
    expect(ask).toHaveBeenCalledTimes(2);
    expect((await loadSettings(store)).rules.min_size).toBe(9);
  });

  it("asks once for several downloads at a time", async () => {
    let answer: (r: NativeResult) => void = () => {};
    const ask = vi.fn(() => new Promise<NativeResult>((r) => (answer = r)));
    const sync = new RulesSync(ask, memory());
    const a = sync.fresh(0);
    const b = sync.fresh(0);
    answer(hello(1));
    await Promise.all([a, b]);
    expect(ask).toHaveBeenCalledTimes(1);
  });

  it("does not hold a download up for an app that is slow to answer", async () => {
    const store = memory();
    let answer: (r: NativeResult) => void = () => {};
    const sync = new RulesSync(() => new Promise<NativeResult>((r) => (answer = r)), store);
    let done = false;
    const waiting = sync.fresh(0).then(() => (done = true));
    await vi.advanceTimersByTimeAsync(RULES_WAIT_MS - 1);
    expect(done).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    await waiting;
    expect(done).toBe(true);
    // The late answer still counts, for the next download.
    answer(hello(7));
    await vi.waitFor(async () => expect((await loadSettings(store)).rules.min_size).toBe(7));
  });

  it("survives a failing hello and an app without rules", async () => {
    const store = memory();
    await editRules(store, { ...DEFAULT_RULES, enabled: false });
    const failing = new RulesSync(() => Promise.reject(new Error("gone")), store);
    expect(await failing.now(0)).toBeNull();
    const notRunning = new RulesSync(
      async () => ({ ...hello(1), app: null, rules: null }) as NativeResult,
      store,
    );
    await notRunning.now(0);
    expect((await loadSettings(store)).rules.enabled).toBe(false);
  });
});
