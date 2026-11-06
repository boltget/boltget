// The context menu of one or more selected downloads: the same actions as
// the row, the details pane and the shortcuts, with the shortcut shown.
// Queue actions (APP-DESIGN, "Queues as built"): start now, wait in the
// queue again, start later, move to another queue and, in a queue's view,
// move within it.
import { t, type PlainMessageKey } from "@boltget/ui/messages";
import { app } from "../app.svelte";
import type { DownloadId } from "../bindings";
import { downloads } from "../engine";
import type { MenuEntry } from "../menu";
import { canPause, canRestart, canResume } from "./actions";
import { queueOfView } from "./model";

export function rowMenu(ids: readonly DownloadId[]): MenuEntry[] {
  const items = ids.flatMap((id) => downloads.get(id) ?? []);
  const states = items.map((d) => d.state);
  const one = items.length === 1 ? items[0] : undefined;
  const entries: MenuEntry[] = [];
  const add = (key: PlainMessageKey, run: () => void, accelerator?: string, enabled = true) =>
    entries.push({ text: t(key), run, accelerator, enabled });

  if (one?.state === "completed") add("action.open", () => app.run("open", ids), "Enter");
  if (states.some((s) => s === "queued")) add("action.start_now", () => app.startNow(ids));
  if (states.some(canPause)) add("action.pause", () => app.run("pause", ids), "Space");
  if (states.some(canResume)) {
    const failed = states.every((s) => s === "failed" || s === "cancelled");
    add(failed ? "action.retry" : "action.resume", () => app.run("resume", ids), "Space");
    add("action.requeue", () => app.requeue(ids));
  }
  if (one?.state === "needs_new_link") add("action.refresh", () => app.run("refresh", ids));
  if (states.some(canRestart)) add("action.restart", () => app.run("restart", ids));
  if (entries.length) entries.push("separator");

  // Queues.
  const waiting = items.filter(
    (d) => d.state !== "completed" && (d.state === "queued" || canResume(d.state)),
  );
  if (waiting.length) add("action.start_later", () => app.startLater(ids));
  if (items.some((d) => d.startAt !== null && d.state !== "completed")) {
    add("action.clear_start", () => app.clearStart(ids));
  }
  const here = new Set(items.map((d) => d.queue));
  entries.push({
    text: t("action.move_to_queue"),
    items: downloads.queues
      .filter((q) => !(here.size === 1 && here.has(q.id)))
      .map((q) => ({ text: app.queueName(q), run: () => app.moveTo(ids, q.id) })),
  });
  if (queueOfView(app.view) !== null && items.some((d) => d.state !== "completed")) {
    const active = app.selection.active ?? undefined;
    add("action.move_up", () => app.shift(ids, "up", active), "Alt+Up");
    add("action.move_down", () => app.shift(ids, "down", active), "Alt+Down");
    add("action.move_top", () => app.shift(ids, "top", active), "Alt+Home");
    add("action.move_bottom", () => app.shift(ids, "bottom", active), "Alt+End");
  }
  entries.push("separator");

  if (one) add("action.show", () => app.run("show", ids), "Ctrl+Enter");
  add("action.copy", () => app.run("copy", ids), "Ctrl+C");
  entries.push("separator");
  add("action.remove", () => app.run("remove", ids), "Delete");
  add("action.delete_menu", () => app.run("delete", ids), "Shift+Delete");
  return entries;
}
