// The context menu of one or more selected downloads: the same actions as
// the row, the details pane and the shortcuts, with the shortcut shown.
import { t, type PlainMessageKey } from "@boltget/ui/messages";
import { app } from "../app.svelte";
import type { DownloadId } from "../bindings";
import { downloads } from "../engine";
import type { MenuEntry } from "../menu";
import { canPause, canRestart, canResume } from "./actions";

export function rowMenu(ids: readonly DownloadId[]): MenuEntry[] {
  const items = ids.flatMap((id) => downloads.get(id) ?? []);
  const states = items.map((d) => d.state);
  const one = items.length === 1 ? items[0] : undefined;
  const entries: MenuEntry[] = [];
  const add = (key: PlainMessageKey, run: () => void, accelerator?: string, enabled = true) =>
    entries.push({ text: t(key), run, accelerator, enabled });

  if (one?.state === "completed") add("action.open", () => app.run("open", ids), "Enter");
  if (states.some(canPause)) add("action.pause", () => app.run("pause", ids), "Space");
  if (states.some(canResume)) {
    const failed = states.every((s) => s === "failed" || s === "cancelled");
    add(failed ? "action.retry" : "action.resume", () => app.run("resume", ids), "Space");
  }
  if (one?.state === "needs_new_link") add("action.refresh", () => app.run("refresh", ids));
  if (states.some(canRestart)) add("action.restart", () => app.run("restart", ids));
  if (entries.length) entries.push("separator");
  if (one) add("action.show", () => app.run("show", ids), "Ctrl+Enter");
  add("action.copy", () => app.run("copy", ids), "Ctrl+C");
  entries.push("separator");
  add("action.remove", () => app.run("remove", ids), "Delete");
  add("action.delete_menu", () => app.run("delete", ids), "Shift+Delete");
  return entries;
}
