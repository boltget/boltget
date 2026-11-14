// Modifier-key bypass and force (docs/BROWSER-INTEGRATION.md, "Modifier-key
// bypass"): the content script reports the keys held on a link click; a
// download of that link within a short window gets them.
import type { Modifiers } from "./capture";

/** How long a click's keys apply (design target: 2 s). */
export const MODIFIER_WINDOW_MS = 2_000;

/** What the content script posts for a link click. */
export interface LinkClick {
  type: "boltget:click";
  href: string;
  alt: boolean;
  shift: boolean;
}

/** Alt alone bypasses Boltget; Alt+Shift forces it; anything else is nothing. */
export function modifiersOf(alt: boolean, shift: boolean): Modifiers {
  if (!alt) return "none";
  return shift ? "force" : "bypass";
}

/** Whether a runtime message is a well-formed {@link LinkClick}. */
export function isLinkClick(message: unknown): message is LinkClick {
  if (typeof message !== "object" || message === null) return false;
  const m = message as Record<string, unknown>;
  return (
    m.type === "boltget:click" &&
    typeof m.href === "string" &&
    m.href.length <= 32 * 1024 &&
    typeof m.alt === "boolean" &&
    typeof m.shift === "boolean"
  );
}

/** Recent link clicks with modifiers, by URL. */
export class ModifierLog {
  readonly #clicks = new Map<string, { modifiers: Modifiers; at: number }>();
  readonly #window: number;

  constructor(window = MODIFIER_WINDOW_MS) {
    this.#window = window;
  }

  /** Records a click; one without modifiers clears an earlier one. */
  record(href: string, modifiers: Modifiers, now: number) {
    this.#prune(now);
    if (modifiers === "none") this.#clicks.delete(href);
    else this.#clicks.set(href, { modifiers, at: now });
    if (this.#clicks.size > 64) {
      const oldest = this.#clicks.keys().next().value;
      if (oldest !== undefined) this.#clicks.delete(oldest);
    }
  }

  /** The keys for a download of any of `urls`, used once. */
  take(urls: (string | null | undefined)[], now: number): Modifiers {
    this.#prune(now);
    for (const url of urls) {
      if (!url) continue;
      const click = this.#clicks.get(url);
      if (click) {
        this.#clicks.delete(url);
        return click.modifiers;
      }
    }
    return "none";
  }

  #prune(now: number) {
    for (const [href, click] of this.#clicks) {
      if (now - click.at > this.#window) this.#clicks.delete(href);
    }
  }
}
