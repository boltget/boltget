// The fallback guarantee (docs/BROWSER-INTEGRATION.md, design goal 1, "Never
// lose a download"): pause the browser's download, ask the app, and drop the
// browser's copy only on `accepted`. Anything else (no app, a hung app, a
// rejection, a timeout, a reply we do not understand) puts the browser's
// download back the way it was: resumed if it can be, started again if the
// pause cost it its connection.
import type { NativeResult } from "./native";

/** The state of a browser download, as `downloads.search` reports it. */
export interface DownloadState {
  state: "in_progress" | "interrupted" | "complete";
  paused: boolean;
  canResume: boolean;
  /** The interrupt reason (`USER_CANCELED`, `NETWORK_FAILED`, ...). */
  error?: string;
  url: string;
}

/** The parts of the `downloads` API a handoff uses, injected for tests. */
export interface DownloadControl {
  pause(id: number): Promise<void>;
  resume(id: number): Promise<void>;
  cancel(id: number): Promise<void>;
  erase(id: number): Promise<void>;
  /** The download now, or null when it is gone (the user removed it). */
  get(id: number): Promise<DownloadState | null>;
  /** Starts the link again as a new browser download. */
  restart(url: string): Promise<void>;
}

/** What happened to the browser's download. */
export type HandoffOutcome =
  /** Boltget took it; the browser's copy was cancelled and erased. */
  | "taken"
  /** Boltget took it, but the browser had already finished its copy (kept). */
  | "taken_both"
  /** Boltget did not take it; the browser's download continues. */
  | "kept"
  /** Boltget did not take it and the paused download was lost: started again. */
  | "restarted"
  /** The download was gone (cancelled or removed by the user meanwhile). */
  | "gone";

/** The result of a handoff, with the reply that decided it. */
export interface Handoff {
  outcome: HandoffOutcome;
  result: NativeResult;
}

async function attempt(action: Promise<void>): Promise<boolean> {
  try {
    await action;
    return true;
  } catch {
    return false;
  }
}

/** Interrupt reasons that mean the user stopped it: never start it again. */
const USER_STOPPED = new Set(["USER_CANCELED", "USER_SHUTDOWN"]);

/** Puts a download the app did not take back into the browser's hands. */
export async function giveBack(
  id: number,
  pausedByUs: boolean,
  control: DownloadControl,
): Promise<HandoffOutcome> {
  const item = await control.get(id).catch(() => null);
  if (!item) return "gone";
  if (item.state === "complete") return "kept";
  if (item.state === "in_progress") {
    if (!item.paused || !pausedByUs) return "kept";
    if (await attempt(control.resume(id))) return "kept";
    await attempt(control.cancel(id));
    await attempt(control.restart(item.url));
    await attempt(control.erase(id));
    return "restarted";
  }
  // Interrupted while it waited (the server dropped the paused connection, say).
  if (item.error && USER_STOPPED.has(item.error)) return "gone";
  if (item.canResume && (await attempt(control.resume(id)))) return "kept";
  await attempt(control.restart(item.url));
  await attempt(control.erase(id));
  return "restarted";
}

/**
 * Hands a browser download to the app. `ask` sends the `capture` message; it
 * resolves with the reply or the failure, never throws (see NativeClient).
 */
export async function handOff(
  id: number,
  ask: () => Promise<NativeResult>,
  control: DownloadControl,
): Promise<Handoff> {
  const pausedByUs = await attempt(control.pause(id));
  let result: NativeResult;
  try {
    result = await ask();
  } catch (error) {
    result = { type: "failed", reason: "other", detail: String(error) };
  }

  if (result.type === "accepted") {
    const item = await control.get(id).catch(() => null);
    if (item?.state === "complete") return { outcome: "taken_both", result };
    await attempt(control.cancel(id));
    await attempt(control.erase(id));
    return { outcome: "taken", result };
  }
  return { outcome: await giveBack(id, pausedByUs, control), result };
}
