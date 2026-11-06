import { t } from "@boltget/ui/messages";
import { describe, expect, it, vi } from "vitest";
import { DEFAULT_SETTINGS } from "../settings/model";
import {
  buildEntries,
  initialEntries,
  step,
  type PaletteActions,
  type PaletteContext,
  type PaletteItem,
} from "./entries";
import { rank } from "./fuzzy";

function context(patch: Partial<PaletteContext> = {}): PaletteContext {
  const actions: PaletteActions = {
    add: vi.fn(),
    pauseAll: vi.fn(),
    resumeAll: vi.fn(),
    settings: vi.fn(),
    search: vi.fn(),
    details: vi.fn(),
    density: vi.fn(),
    view: vi.fn(),
    limit: vi.fn(),
    folder: vi.fn(),
    quit: vi.fn(),
    update: vi.fn(),
    act: vi.fn(),
    reveal: vi.fn(),
    toggleQueue: vi.fn(),
    editQueue: vi.fn(),
    startNow: vi.fn(),
    startLater: vi.fn(),
    moveTo: vi.fn(),
  };
  return {
    settings: DEFAULT_SETTINGS,
    compact: false,
    selection: [],
    downloads: [],
    queues: [
      { id: 1, name: "Main", started: true },
      { id: 2, name: "Night", started: false },
    ],
    stateLabel: (s) => s,
    do: actions,
    ...patch,
  };
}

const find = (ctx: PaletteContext, query: string) =>
  rank(buildEntries(ctx), query, (e) => ({ label: e.label, synonyms: e.synonyms }));

describe("queue commands", () => {
  it("start or stop each queue, open its settings and make new ones", () => {
    const ctx = context();
    const entries = buildEntries(ctx);
    const labels = entries.map((e) => e.label);
    expect(labels).toContain("Stop queue Main");
    expect(labels).toContain("Start queue Night");
    expect(labels).toContain("Queue settings: Night");
    expect(labels).toContain("Go to Night");
    entries.find((e) => e.label === "Start queue Night")?.run();
    expect(ctx.do.toggleQueue).toHaveBeenCalledWith(2);
    entries.find((e) => e.id === "queue.new")?.run();
    expect(ctx.do.editQueue).toHaveBeenCalledWith(null);
    expect(find(ctx, "start night")[0]?.label).toBe("Start queue Night");
    expect(find(ctx, "line night").map((e) => e.label)).toContain("Start queue Night");
  });

  it("move the selection to another queue, start it now or later", () => {
    const ctx = context({
      selection: [{ id: 7, name: "a.iso", state: "queued", queue: 1 }],
    });
    const entries = buildEntries(ctx);
    const labels = entries.filter((e) => e.group === "selection").map((e) => e.label);
    expect(labels).toContain("Move to Night");
    expect(labels).not.toContain("Move to Main");
    expect(labels).toContain(t("action.start_now"));
    entries.find((e) => e.label === "Move to Night")?.run();
    expect(ctx.do.moveTo).toHaveBeenCalledWith([7], 2);
    entries.find((e) => e.id === "selection.start_later")?.run();
    expect(ctx.do.startLater).toHaveBeenCalledWith([7]);
  });
});

describe("palette entries", () => {
  it("shows actions with their shortcuts", () => {
    const entries = buildEntries(context());
    const add = entries.find((e) => e.id === "add");
    expect(add?.label).toBe(t("command.add"));
    expect(add?.shortcut).toBe("Ctrl+N");
    expect(entries.find((e) => e.id === "pause_all")?.shortcut).toBe("Ctrl+Shift+P");
    expect(entries.find((e) => e.id === "settings")?.shortcut).toBe("Ctrl+,");
  });

  it("finds the speed limit by a synonym and sets it", () => {
    const ctx = context();
    const [first] = find(ctx, "throttle 5");
    expect(first?.id).toBe("limit.5");
    first?.run();
    expect(ctx.do.limit).toHaveBeenCalledWith(5 * 1024 * 1024);
    // No "remove the limit" without a limit; the current preset is not offered.
    expect(find(ctx, "remove speed limit").some((e) => e.id === "limit.none")).toBe(false);
    const limited = context({ settings: { ...DEFAULT_SETTINGS, speedLimit: 5 * 1024 * 1024 } });
    const ids = buildEntries(limited).map((e) => e.id);
    expect(ids).toContain("limit.none");
    expect(ids).not.toContain("limit.5");
  });

  it("offers the other themes and flips switches", () => {
    const ctx = context();
    const ids = buildEntries(ctx).map((e) => e.id);
    expect(ids).toEqual(expect.arrayContaining(["theme.light", "theme.dark"]));
    expect(ids).not.toContain("theme.system");
    const clipboard = buildEntries(ctx).find((e) => e.id === "toggle.clipboard");
    expect(clipboard?.label).toBe(t("command.clipboard.on"));
    clipboard?.run();
    expect(ctx.do.update).toHaveBeenCalledWith({ clipboard: true });
    clipboard?.reveal?.();
    expect(ctx.do.settings).toHaveBeenCalledWith("clipboard");
  });

  it("acts on the selection only with what applies", () => {
    const one: PaletteItem = { id: 7, name: "ubuntu.iso", state: "downloading" };
    const ctx = context({ selection: [one] });
    const selection = buildEntries(ctx).filter((e) => e.group === "selection");
    const ids = selection.map((e) => e.id);
    expect(ids).toContain("selection.pause");
    expect(ids).not.toContain("selection.resume");
    expect(ids).not.toContain("selection.open");
    expect(selection[0]?.hint).toBe("ubuntu.iso");
    selection.find((e) => e.id === "selection.pause")?.run();
    expect(ctx.do.act).toHaveBeenCalledWith("pause", [7]);
    const done = context({ selection: [{ ...one, state: "completed" }] });
    expect(buildEntries(done).find((e) => e.id === "selection.open")?.shortcut).toBe("Enter");
    // Nothing selected: no selection group.
    expect(buildEntries(context()).some((e) => e.group === "selection")).toBe(false);
  });

  it("reaches every download, newest first, and every setting", () => {
    const downloads: PaletteItem[] = [
      { id: 1, name: "old.zip", state: "completed" },
      { id: 2, name: "new.mkv", state: "paused" },
    ];
    const ctx = context({ downloads });
    const entries = buildEntries(ctx);
    const files = entries.filter((e) => e.group === "downloads");
    expect(files.map((e) => e.label)).toEqual(["new.mkv", "old.zip"]);
    files[0]?.run();
    expect(ctx.do.reveal).toHaveBeenCalledWith(2);
    const [proxy] = find(ctx, "proxy");
    expect(proxy?.id).toBe("setting.proxy");
    expect(proxy?.hint).toBe(t("settings.section.network"));
    // Before typing: no settings rows and no downloads.
    expect(initialEntries(entries).some((e) => e.group === "downloads")).toBe(false);
    expect(initialEntries(entries).some((e) => e.group === "settings")).toBe(false);
  });

  it("goes to views and categories", () => {
    const ctx = context();
    const [video] = find(ctx, "go video");
    expect(video?.id).toBe("go.video");
    video?.run();
    expect(ctx.do.view).toHaveBeenCalledWith("video");
  });
});

describe("palette keyboard", () => {
  it("moves through results and wraps", () => {
    expect(step(-1, 1, 3)).toBe(0);
    expect(step(-1, -1, 3)).toBe(2);
    expect(step(0, 1, 3)).toBe(1);
    expect(step(2, 1, 3)).toBe(0);
    expect(step(0, -1, 3)).toBe(2);
    expect(step(1, 10, 3)).toBe(2);
    expect(step(0, 1, 0)).toBe(-1);
  });
});
