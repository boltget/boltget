// What the command palette offers (APP-DESIGN #20): every action with its
// shortcut, the selected downloads' actions, views and categories, theme and
// speed limit presets, every setting, and every download. Built from a plain
// context so it can be tested without the app.
import { t, type PlainMessageKey } from "@boltget/ui/messages";
import type {
  AppSettings,
  DownloadId,
  DownloadState,
  QueueId,
  SettingsPatch,
  ThemeSetting,
} from "../bindings";
import type { IconName } from "../Icon.svelte";
import { canPause, canRestart, canResume, type ActionId } from "../list/actions";
import { CATEGORIES, CATEGORY_LABELS, categoryIcon } from "../list/category";
import { queueView, type View } from "../list/model";
import { ROWS, SECTIONS, type RowId } from "../settings/registry";
import { keysOf, type ShortcutId } from "../shortcuts";

export type Group = "selection" | "actions" | "go" | "settings" | "downloads";

/** The order groups appear in when nothing is typed. */
export const GROUPS: readonly { id: Group; label: PlainMessageKey }[] = [
  { id: "selection", label: "palette.group.selection" },
  { id: "actions", label: "palette.group.actions" },
  { id: "go", label: "palette.group.go" },
  { id: "settings", label: "palette.group.settings" },
  { id: "downloads", label: "palette.group.downloads" },
];

export interface PaletteEntry {
  /** Unique within one palette. */
  id: string;
  group: Group;
  label: string;
  /** Muted text after the label: a file name, a section, a state. */
  hint?: string;
  /** Other words that find it ("throttle" for a speed limit). */
  synonyms?: string;
  /** Keycap text of its shortcut. */
  shortcut?: string;
  icon: IconName;
  /** Enter. */
  run: () => void;
  /** Tab: open it in context (the setting's row, the download in the list). */
  reveal?: () => void;
}

export interface PaletteItem {
  id: DownloadId;
  name: string;
  state: DownloadState;
  queue?: QueueId;
}

/** A queue as the palette offers it. */
export interface PaletteQueue {
  id: QueueId;
  name: string;
  started: boolean;
}

/** What the palette can do; the app passes its own actions. */
export interface PaletteActions {
  add(): void;
  pauseAll(): void;
  resumeAll(): void;
  settings(row?: RowId): void;
  search(): void;
  details(): void;
  density(): void;
  view(view: View): void;
  limit(bytesPerSecond: number | null): void;
  folder(): void;
  quit(): void;
  update(patch: SettingsPatch): void;
  act(action: ActionId, ids: DownloadId[]): void;
  /** Selects a download and shows its details. */
  reveal(id: DownloadId): void;
  /** Queues: start or stop one, open its settings (`null`: a new one). */
  toggleQueue(id: QueueId): void;
  editQueue(id: QueueId | null): void;
  /** Downloads: start now, choose a time, move to a queue. */
  startNow(ids: DownloadId[]): void;
  startLater(ids: DownloadId[]): void;
  moveTo(ids: DownloadId[], queue: QueueId): void;
}

export interface PaletteContext {
  settings: AppSettings;
  compact: boolean;
  selection: readonly PaletteItem[];
  downloads: readonly PaletteItem[];
  queues: readonly PaletteQueue[];
  stateLabel: (state: DownloadState) => string;
  do: PaletteActions;
}

/** The speed limit presets, in MiB/s (the tray offers the same). */
export const LIMIT_PRESETS = [1, 5, 10, 25, 50] as const;
const MIB = 1024 * 1024;

/** Downloads offered at most, so a long list stays quick. */
const MAX_DOWNLOADS = 500;

const VIEWS: readonly { view: View; label: PlainMessageKey; icon: IconName }[] = [
  { view: "all", label: "view.all", icon: "all" },
  { view: "active", label: "view.active", icon: "active" },
  { view: "completed", label: "view.completed", icon: "completed" },
  { view: "scheduled", label: "view.scheduled", icon: "scheduled" },
  ...CATEGORIES.map((c) => ({
    view: c,
    label: CATEGORY_LABELS[c],
    icon: (c === "other" ? "other" : c) as IconName,
  })),
];

const keys = (id: ShortcutId) => t(keysOf(id));

export function buildEntries(ctx: PaletteContext): PaletteEntry[] {
  const out: PaletteEntry[] = [];
  const d = ctx.do;
  const s = ctx.settings;

  // The selection --------------------------------------------------------
  const selected = ctx.selection;
  if (selected.length > 0) {
    const ids = selected.map((i) => i.id);
    const states = selected.map((i) => i.state);
    const one = selected.length === 1 ? selected[0] : undefined;
    const hint = one ? one.name : t("command.selection", { count: selected.length, name: "" });
    const act = (action: ActionId, label: PlainMessageKey, icon: IconName, shortcut?: ShortcutId) =>
      out.push({
        id: `selection.${action}`,
        group: "selection",
        label: t(label),
        hint,
        icon,
        shortcut: shortcut && keys(shortcut),
        run: () => d.act(action, ids),
        reveal: one ? () => d.reveal(one.id) : undefined,
      });
    if (one?.state === "completed") act("open", "action.open", "open", "open");
    if (states.some(canPause)) act("pause", "action.pause", "pause", "toggle");
    if (states.some(canResume)) {
      const failed = states.every((st) => st === "failed" || st === "cancelled");
      act("resume", failed ? "action.retry" : "action.resume", "play", "toggle");
    }
    if (one?.state === "needs_new_link") act("refresh", "action.refresh", "link");
    if (states.some(canRestart)) act("restart", "action.restart", "retry");
    const queued = (run: () => void, id: string, label: string, icon: IconName) =>
      out.push({
        id: `selection.${id}`,
        group: "selection",
        label,
        hint,
        icon,
        synonyms: t("keywords.queue"),
        run,
      });
    if (states.some((st) => st === "queued")) {
      queued(() => d.startNow(ids), "start_now", t("action.start_now"), "play");
    }
    if (states.some((st) => st === "queued" || canResume(st))) {
      queued(() => d.startLater(ids), "start_later", t("action.start_later"), "scheduled");
    }
    const here = new Set(selected.map((i) => i.queue));
    for (const q of ctx.queues) {
      if (here.size === 1 && here.has(q.id)) continue;
      queued(
        () => d.moveTo(ids, q.id),
        `move.${q.id}`,
        t("command.move_to", { queue: q.name }),
        "queue",
      );
    }
    if (one) act("show", "action.show", "folder", "show");
    act("copy", "action.copy", "copy", "copy");
    act("remove", "action.remove", "trash", "remove");
    act("delete", "action.delete_menu", "trash", "delete");
  }

  // Actions --------------------------------------------------------------
  const action = (
    id: string,
    label: string,
    icon: IconName,
    run: () => void,
    extra: Partial<PaletteEntry> = {},
  ) => out.push({ id, group: "actions", label, icon, run, ...extra });
  action("add", t("command.add"), "plus", d.add, {
    shortcut: keys("add"),
    synonyms: t("keywords.add"),
  });
  action("pause_all", t("command.pause_all"), "pause", d.pauseAll, {
    shortcut: keys("pauseAll"),
    synonyms: t("keywords.pause_all"),
  });
  action("resume_all", t("command.resume_all"), "play", d.resumeAll, {
    shortcut: keys("resumeAll"),
    synonyms: t("keywords.resume_all"),
  });
  action("search", t("command.search"), "search", d.search, {
    shortcut: keys("search"),
    synonyms: t("keywords.search"),
  });
  action("details", t("command.details"), "pane", d.details, {
    shortcut: keys("details"),
    synonyms: t("keywords.details"),
  });
  action(
    "density",
    t(ctx.compact ? "command.density.comfortable" : "command.density.compact"),
    "all",
    d.density,
    { shortcut: keys("density"), synonyms: t("keywords.density") },
  );
  action("folder", t("command.folder"), "folder", d.folder, { synonyms: t("keywords.folder") });
  action("settings", t("command.settings"), "settings", () => d.settings(), {
    shortcut: keys("settings"),
    synonyms: t("keywords.settings"),
  });
  // Speed limit presets: the current one is not offered again.
  if (s.speedLimit !== null) {
    action("limit.none", t("command.limit.none"), "active", () => d.limit(null), {
      synonyms: t("keywords.limit"),
    });
  }
  for (const mib of LIMIT_PRESETS) {
    if (s.speedLimit === mib * MIB) continue;
    action(
      `limit.${mib}`,
      t("command.limit", { speed: t("speed.mib", { value: mib }) }),
      "active",
      () => d.limit(mib * MIB),
      { synonyms: t("keywords.limit") },
    );
  }
  // Themes: the other two.
  const themes: [ThemeSetting, PlainMessageKey][] = [
    ["system", "command.theme.system"],
    ["light", "command.theme.light"],
    ["dark", "command.theme.dark"],
  ];
  for (const [theme, label] of themes) {
    if (theme === s.theme) continue;
    action(`theme.${theme}`, t(label), "settings", () => d.update({ theme }), {
      synonyms: t("keywords.theme"),
      reveal: () => d.settings("theme"),
    });
  }
  // Switches, worded as what they do now.
  const toggle = (
    id: RowId,
    on: boolean,
    labels: [PlainMessageKey, PlainMessageKey],
    synonyms: PlainMessageKey,
    patch: (value: boolean) => SettingsPatch,
  ) =>
    action(`toggle.${id}`, t(on ? labels[1] : labels[0]), "settings", () => d.update(patch(!on)), {
      synonyms: t(synonyms),
      reveal: () => d.settings(id),
    });
  toggle(
    "clipboard",
    s.clipboard,
    ["command.clipboard.on", "command.clipboard.off"],
    "keywords.clipboard",
    (clipboard) => ({ clipboard }),
  );
  toggle(
    "notifications",
    s.notifications,
    ["command.notifications.on", "command.notifications.off"],
    "keywords.notifications",
    (notifications) => ({ notifications }),
  );
  toggle(
    "autostart",
    s.startWithWindows,
    ["command.autostart.on", "command.autostart.off"],
    "keywords.autostart",
    (startWithWindows) => ({ startWithWindows }),
  );
  toggle(
    "tray",
    s.closeToTray,
    ["command.tray.on", "command.tray.off"],
    "keywords.tray",
    (closeToTray) => ({ closeToTray }),
  );
  // Queues: start or stop each, their settings, a new one.
  for (const q of ctx.queues) {
    action(
      `queue.toggle.${q.id}`,
      t(q.started ? "command.queue.stop" : "command.queue.start", { name: q.name }),
      q.started ? "stop" : "play",
      () => d.toggleQueue(q.id),
      { synonyms: t("keywords.queue") },
    );
    action(
      `queue.settings.${q.id}`,
      t("command.queue.settings", { name: q.name }),
      "settings",
      () => d.editQueue(q.id),
      { synonyms: t("keywords.queue_settings") },
    );
  }
  action("queue.new", t("command.queue.new"), "plus", () => d.editQueue(null), {
    synonyms: t("keywords.queue"),
  });
  action("quit", t("command.quit"), "close", d.quit, { synonyms: t("keywords.quit") });

  // Views and categories -------------------------------------------------
  for (const v of VIEWS) {
    out.push({
      id: `go.${v.view}`,
      group: "go",
      label: t("command.go", { view: t(v.label) }),
      icon: v.icon,
      run: () => d.view(v.view),
    });
  }

  for (const q of ctx.queues) {
    out.push({
      id: `go.queue.${q.id}`,
      group: "go",
      label: t("command.go", { view: q.name }),
      icon: "queue",
      synonyms: t("keywords.queue"),
      run: () => d.view(queueView(q.id)),
    });
  }

  // Settings rows --------------------------------------------------------
  for (const row of ROWS) {
    const section = SECTIONS.find((sec) => sec.id === row.section);
    out.push({
      id: `setting.${row.id}`,
      group: "settings",
      label: t(row.label),
      hint: section && t(section.label),
      synonyms: row.keywords && t(row.keywords),
      icon: section?.icon ?? "settings",
      run: () => d.settings(row.id),
      reveal: () => d.settings(row.id),
    });
  }

  // Downloads ------------------------------------------------------------
  for (const item of ctx.downloads.slice(-MAX_DOWNLOADS).reverse()) {
    out.push({
      id: `download.${item.id}`,
      group: "downloads",
      label: item.name,
      hint: ctx.stateLabel(item.state),
      icon: categoryIcon(item.name),
      run: () => d.reveal(item.id),
      reveal: () => d.reveal(item.id),
    });
  }
  return out;
}

/** What shows before anything is typed: everything but settings rows and downloads. */
export function initialEntries(entries: readonly PaletteEntry[]): PaletteEntry[] {
  return entries.filter((e) => e.group !== "settings" && e.group !== "downloads");
}

/** The next index when moving by `delta` through `count` results, wrapping. */
export function step(index: number, delta: number, count: number): number {
  if (count === 0) return -1;
  if (index < 0) return delta >= 0 ? 0 : count - 1;
  return (((index + delta) % count) + count) % count;
}
