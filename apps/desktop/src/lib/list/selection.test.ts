import { describe, expect, it } from "vitest";
import { canPause, canRestart, canResume, primaryAction, toggleIntent } from "./actions";
import {
  EMPTY,
  click,
  contextClick,
  move,
  prune,
  selectAll,
  selectedInOrder,
  type Selection,
} from "./selection";

const ORDER = [10, 20, 30, 40, 50];
const ids = (s: Selection) => [...s.ids].sort((a, b) => a - b);

describe("clicks", () => {
  it("select one, toggle with Ctrl and extend with Shift", () => {
    let s = click(EMPTY, ORDER, 20);
    expect(ids(s)).toEqual([20]);
    s = click(s, ORDER, 40, { shift: true });
    expect(ids(s)).toEqual([20, 30, 40]);
    expect(s.anchor).toBe(20);
    s = click(s, ORDER, 10, { shift: true });
    expect(ids(s)).toEqual([10, 20]);
    s = click(s, ORDER, 50, { ctrl: true });
    expect(ids(s)).toEqual([10, 20, 50]);
    s = click(s, ORDER, 20, { ctrl: true });
    expect(ids(s)).toEqual([10, 50]);
    expect(s.active).toBe(20);
  });

  it("add a range to the selection with Ctrl+Shift", () => {
    let s = click(EMPTY, ORDER, 10);
    s = click(s, ORDER, 30, { ctrl: true });
    s = click(s, ORDER, 50, { ctrl: true, shift: true });
    expect(ids(s)).toEqual([10, 30, 40, 50]);
  });

  it("keep a selection when right-clicking inside it", () => {
    const s = click(click(EMPTY, ORDER, 10), ORDER, 30, { shift: true });
    expect(ids(contextClick(s, 20))).toEqual([10, 20, 30]);
    expect(ids(contextClick(s, 50))).toEqual([50]);
  });
});

describe("keyboard", () => {
  it("moves and clamps at the ends", () => {
    let s = move(EMPTY, ORDER, 1);
    expect(s.active).toBe(10);
    s = move(s, ORDER, 1);
    expect(ids(s)).toEqual([20]);
    s = move(s, ORDER, 99);
    expect(s.active).toBe(50);
    s = move(s, ORDER, "home");
    expect(s.active).toBe(10);
    s = move(s, ORDER, "end");
    expect(s.active).toBe(50);
    expect(move(EMPTY, ORDER, -1).active).toBe(50);
    expect(move(EMPTY, [], 1)).toBe(EMPTY);
  });

  it("extends with Shift from the anchor", () => {
    let s = move(click(EMPTY, ORDER, 30), ORDER, 1, { shift: true });
    expect(ids(s)).toEqual([30, 40]);
    s = move(s, ORDER, -2, { shift: true });
    expect(ids(s)).toEqual([20, 30]);
    s = move(s, ORDER, "end", { shift: true });
    expect(ids(s)).toEqual([30, 40, 50]);
  });

  it("moves the keyboard's row without selecting with Ctrl", () => {
    const s = move(click(EMPTY, ORDER, 30), ORDER, 1, { ctrl: true });
    expect(ids(s)).toEqual([30]);
    expect(s.active).toBe(40);
  });

  it("selects all with Ctrl+A", () => {
    expect(ids(selectAll(EMPTY, ORDER))).toEqual(ORDER);
    expect(selectAll(EMPTY, [])).toBe(EMPTY);
  });
});

describe("pruning", () => {
  it("follows a removed row to the one that took its place", () => {
    const s = click(EMPTY, ORDER, 30);
    const next = prune(s, [10, 20, 40, 50], ORDER);
    expect(next.active).toBe(40);
    expect(ids(next)).toEqual([40]);
    const last = prune(click(EMPTY, ORDER, 50), [10, 20, 30, 40], ORDER);
    expect(last.active).toBe(40);
  });

  it("lets go of rows a filter hides", () => {
    const s = click(EMPTY, ORDER, 30);
    const next = prune(s, [10, 20], ORDER, false);
    expect(next.active).toBeNull();
    expect(next.ids.size).toBe(0);
  });

  it("returns the same selection when nothing changed", () => {
    const s = click(EMPTY, ORDER, 30);
    expect(prune(s, ORDER, ORDER)).toBe(s);
  });

  it("lists the selection in display order", () => {
    const s = click(click(EMPTY, ORDER, 50), ORDER, 10, { ctrl: true });
    expect(selectedInOrder(s, ORDER)).toEqual([10, 50]);
  });
});

describe("actions by state", () => {
  it("offer one primary action per state", () => {
    expect(primaryAction("downloading")).toBe("pause");
    expect(primaryAction("queued")).toBe("pause");
    expect(primaryAction("paused")).toBe("resume");
    expect(primaryAction("failed")).toBe("retry");
    expect(primaryAction("needs_new_link")).toBe("refresh");
    expect(primaryAction("corrupt")).toBe("restart");
    expect(primaryAction("completed")).toBe("open");
  });

  it("pause, resume and restart only where the engine allows it", () => {
    expect(canPause("downloading")).toBe(true);
    expect(canPause("verifying")).toBe(false);
    expect(canResume("paused")).toBe(true);
    expect(canResume("needs_new_link")).toBe(false);
    expect(canRestart("completed")).toBe(false);
    expect(canRestart("corrupt")).toBe(true);
  });

  it("Space pauses when anything runs, else resumes", () => {
    expect(toggleIntent(["downloading", "paused"])).toBe("pause");
    expect(toggleIntent(["paused", "failed"])).toBe("resume");
    expect(toggleIntent(["completed"])).toBeNull();
  });
});
