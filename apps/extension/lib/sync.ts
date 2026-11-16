// Keeping the app's capture rules current in the background
// (docs/BROWSER-INTEGRATION.md, "Capture rules"). The app sends its rules
// only in answer to `hello`, so the background asks at start and, before it
// decides about a download, again when its last answer is older than
// RULES_FRESH_MS: a change made in the app's Settings applies to the next
// download, not only after the popup or options open. The download waits
// for that answer at most RULES_WAIT_MS, then decides with the rules it has.
import type { NativeResult } from "./native";
import { adoptAppRules, type Store } from "./settings";

/** How long an answer counts as current. */
export const RULES_FRESH_MS = 30_000;
/** How long a download waits for a fresh answer. */
export const RULES_WAIT_MS = 1_000;

export class RulesSync {
  readonly #hello: () => Promise<NativeResult>;
  readonly #store: Store;
  #asked = Number.NEGATIVE_INFINITY;
  #pending: Promise<NativeResult | null> | null = null;

  constructor(hello: () => Promise<NativeResult>, store: Store) {
    this.#hello = hello;
    this.#store = store;
  }

  /** Says hello at `now` and takes the app's rules when it sends them. */
  now(now: number): Promise<NativeResult | null> {
    if (this.#pending) return this.#pending;
    this.#asked = now;
    const run = this.#run();
    this.#pending = run;
    void run.finally(() => {
      if (this.#pending === run) this.#pending = null;
    });
    return run;
  }

  /**
   * Before a decision at `now`: asks again when the last hello is older than
   * RULES_FRESH_MS, and waits for the answer at most `wait` ms.
   */
  async fresh(now: number, wait = RULES_WAIT_MS): Promise<void> {
    if (now - this.#asked < RULES_FRESH_MS) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const late = new Promise<void>((resolve) => {
      timer = setTimeout(resolve, wait);
    });
    await Promise.race([this.now(now), late]);
    clearTimeout(timer);
  }

  async #run(): Promise<NativeResult | null> {
    try {
      const result = await this.#hello();
      if (result.type === "hello" && result.rules) await adoptAppRules(this.#store, result.rules);
      return result;
    } catch {
      return null;
    }
  }
}
