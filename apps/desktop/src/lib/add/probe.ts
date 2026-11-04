// The add sheet's probes (APP-DESIGN #6): links are probed as they are typed
// or pasted, debounced while typing, at most one at a time for a single link
// (a few for a list), each link once. The backend cannot abort a probe, so
// "cancel" means its answer is not shown: a link no longer in the field is
// skipped when its turn comes, and a closed sheet ignores what arrives.
import type { ProbePreview, UiError } from "../bindings";
import type { Result } from "../engine";

/** How long typing pauses before the link is probed. */
export const DEBOUNCE_MS = 350;
/** Probes at once for a list of links; a single link has one. */
export const LIST_CONCURRENCY = 3;

export type ProbeEntry =
  | { status: "probing"; since: number }
  | { status: "ready"; preview: ProbePreview }
  | { status: "failed"; error: UiError };

export interface ProbeQueueOptions {
  probe: (url: string) => Promise<Result<ProbePreview>>;
  /** Called whenever an entry changes. */
  onChange: () => void;
  debounceMs?: number;
  now?: () => number;
}

export class ProbeQueue {
  readonly #probe: ProbeQueueOptions["probe"];
  readonly #onChange: () => void;
  readonly #debounce: number;
  readonly #now: () => number;
  readonly #entries = new Map<string, ProbeEntry>();
  readonly #inFlight = new Set<string>();
  #wanted: string[] = [];
  #timer: ReturnType<typeof setTimeout> | null = null;
  #disposed = false;

  constructor(options: ProbeQueueOptions) {
    this.#probe = options.probe;
    this.#onChange = options.onChange;
    this.#debounce = options.debounceMs ?? DEBOUNCE_MS;
    this.#now = options.now ?? Date.now;
  }

  /** What is known about a link (nothing until its probe starts). */
  get(url: string): ProbeEntry | undefined {
    return this.#entries.get(url);
  }

  /** How many probes are running. */
  get running(): number {
    return this.#inFlight.size;
  }

  /**
   * The links in the field now. Typing waits for a pause; `now` (a paste, a
   * drop, a prefilled sheet) starts at once.
   */
  want(urls: readonly string[], now = false): void {
    if (this.#disposed) return;
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = null;
    if (now || urls.length === 0) {
      this.#wanted = [...urls];
      this.#pump();
      return;
    }
    this.#timer = setTimeout(() => {
      this.#timer = null;
      this.#wanted = [...urls];
      this.#pump();
    }, this.#debounce);
  }

  /** Probes a link again (after it failed). */
  retry(url: string): void {
    if (this.#disposed || this.#inFlight.has(url)) return;
    this.#entries.delete(url);
    this.#onChange();
    this.#pump();
  }

  /** Stops: pending links are dropped and late answers ignored. */
  dispose(): void {
    this.#disposed = true;
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = null;
    this.#wanted = [];
  }

  #pump(): void {
    const limit = this.#wanted.length > 1 ? LIST_CONCURRENCY : 1;
    while (this.#inFlight.size < limit) {
      const next = this.#wanted.find((u) => !this.#entries.has(u) && !this.#inFlight.has(u));
      if (next === undefined) return;
      void this.#start(next);
    }
  }

  async #start(url: string): Promise<void> {
    this.#inFlight.add(url);
    this.#entries.set(url, { status: "probing", since: this.#now() });
    this.#onChange();
    let result: Result<ProbePreview>;
    try {
      result = await this.#probe(url);
    } catch (e) {
      result = { status: "error", error: { key: "error.unknown", message: String(e) } };
    }
    this.#inFlight.delete(url);
    if (this.#disposed) return;
    this.#entries.set(
      url,
      result.status === "ok"
        ? { status: "ready", preview: result.data }
        : { status: "failed", error: result.error },
    );
    this.#onChange();
    this.#pump();
  }
}

/** Probe errors that mean "the server could not be reached", worded as one. */
const UNREACHABLE = new Set(["error.network", "error.timeout", "error.tls"]);
/** Errors that mean "sign in first". */
const AUTH = new Set(["error.http.unauthorized", "error.http.forbidden", "error.http.proxy_auth"]);

export type ProbeProblem = "unreachable" | "auth" | "other";

/** How the sheet words a failed probe. */
export function problemOf(error: Pick<UiError, "key">): ProbeProblem {
  if (UNREACHABLE.has(error.key)) return "unreachable";
  if (AUTH.has(error.key)) return "auth";
  return "other";
}
