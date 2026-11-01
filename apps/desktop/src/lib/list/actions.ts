// Which actions a download offers in its state: the row's primary action,
// the context menu and the details pane all ask here, so they agree.
import type { DownloadState } from "../bindings";
import { isRunning } from "./model";

export type ActionId =
  | "pause"
  | "resume"
  | "retry"
  | "restart"
  | "refresh"
  | "open"
  | "show"
  | "copy"
  | "remove"
  | "delete";

/** The one action a row shows at its right on hover or focus. */
export function primaryAction(state: DownloadState): ActionId {
  switch (state) {
    case "completed":
      return "open";
    case "failed":
    case "cancelled":
      return "retry";
    case "needs_new_link":
      return "refresh";
    case "corrupt":
      return "restart";
    case "paused":
      return "resume";
    default:
      return "pause";
  }
}

export function canPause(state: DownloadState): boolean {
  return state === "queued" || (isRunning(state) && state !== "verifying");
}

export function canResume(state: DownloadState): boolean {
  return state === "paused" || state === "failed" || state === "cancelled";
}

/** Starting again from zero: any stopped download that is not complete. */
export function canRestart(state: DownloadState): boolean {
  return (
    state === "paused" ||
    state === "failed" ||
    state === "cancelled" ||
    state === "corrupt" ||
    state === "needs_new_link"
  );
}

/**
 * Space on a selection: pause when anything selected can pause, else
 * resume what can resume; `null` when neither applies.
 */
export function toggleIntent(states: readonly DownloadState[]): "pause" | "resume" | null {
  if (states.some(canPause)) return "pause";
  if (states.some(canResume)) return "resume";
  return null;
}
