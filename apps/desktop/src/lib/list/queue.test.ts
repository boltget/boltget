import { describe, expect, it } from "vitest";
import {
  dateTimeValue,
  dropBefore,
  inQueue,
  nextHour,
  parseDateTime,
  parseTime,
  placeOrder,
  queueCounts,
  queueOrder,
  shiftSelection,
  timeValue,
  type QueueItem,
} from "./queue";

const item = (id: number, queuePosition: number, extra: Partial<QueueItem> = {}): QueueItem => ({
  id,
  state: "queued",
  queue: 1,
  queuePosition,
  ...extra,
});

describe("queue order", () => {
  it("lists a queue's unfinished downloads by place", () => {
    const items = [
      item(1, 3),
      item(2, 1),
      item(3, 2, { state: "completed" }),
      item(4, 2, { queue: 2 }),
      item(5, 1),
    ];
    expect(queueOrder(items, 1)).toEqual([2, 5, 1]);
    expect(queueOrder(items, 2)).toEqual([4]);
    expect(inQueue(item(3, 1, { state: "completed" }), 1)).toBe(false);
  });

  it("places moved downloads like the engine does", () => {
    const order = [1, 2, 3, 4, 5];
    expect(placeOrder(order, [4, 2], 1)).toEqual([4, 2, 1, 3, 5]);
    expect(placeOrder(order, [1], null)).toEqual([2, 3, 4, 5, 1]);
    expect(placeOrder(order, [2, 3], 3)).toEqual([1, 4, 5, 2, 3]);
    expect(placeOrder(order, [9], 2)).toEqual([1, 9, 2, 3, 4, 5]);
  });

  it("shifts a selection as one block, and says when nothing moves", () => {
    const order = [1, 2, 3, 4, 5];
    const sel = (...ids: number[]) => new Set(ids);
    expect(shiftSelection(order, sel(3), "up")).toEqual({ ids: [3], before: 2 });
    expect(shiftSelection(order, sel(3), "down")).toEqual({ ids: [3], before: 5 });
    expect(shiftSelection(order, sel(4), "down")).toEqual({ ids: [4], before: null });
    expect(shiftSelection(order, sel(1), "up")).toBeNull();
    expect(shiftSelection(order, sel(5), "down")).toBeNull();
    expect(shiftSelection(order, sel(3, 4), "up")).toEqual({ ids: [3, 4], before: 2 });
    expect(shiftSelection(order, sel(4, 5), "top")).toEqual({ ids: [4, 5], before: 1 });
    expect(shiftSelection(order, sel(1, 2), "top")).toBeNull();
    expect(shiftSelection(order, sel(2), "bottom")).toEqual({ ids: [2], before: null });
    // A scattered selection at the top gathers there.
    expect(shiftSelection(order, sel(1, 3), "up")).toEqual({ ids: [1, 3], before: 2 });
    expect(shiftSelection(order, sel(9), "up")).toBeNull();
    // Applying a shift gives the order the user expects.
    const up = shiftSelection(order, sel(3, 4), "up");
    expect(up && placeOrder(order, up.ids, up.before)).toEqual([1, 3, 4, 2, 5]);
    const down = shiftSelection(order, sel(2, 3), "down");
    expect(down && placeOrder(order, down.ids, down.before)).toEqual([1, 4, 2, 3, 5]);
  });

  it("drops before the first download at the slot that is not dragged", () => {
    const order = [1, 2, 3, 4];
    const dragged = new Set([2, 3]);
    expect(dropBefore(order, 0, dragged)).toBe(1);
    expect(dropBefore(order, 1, dragged)).toBe(4);
    expect(dropBefore(order, 2, dragged)).toBe(4);
    expect(dropBefore(order, 4, dragged)).toBeNull();
    expect(dropBefore(order, -1, new Set())).toBe(1);
  });

  it("counts unfinished downloads per queue", () => {
    const counts = queueCounts([
      item(1, 1),
      item(2, 2, { state: "paused" }),
      item(3, 3, { state: "completed" }),
      item(4, 1, { queue: 2 }),
    ]);
    expect(counts.get(1)).toBe(2);
    expect(counts.get(2)).toBe(1);
    expect(counts.get(3)).toBeUndefined();
  });
});

describe("times", () => {
  it("reads and writes daily times", () => {
    expect(timeValue(null)).toBe("");
    expect(timeValue(7 * 60 + 5)).toBe("07:05");
    expect(parseTime("23:30")).toBe(23 * 60 + 30);
    expect(parseTime("7:05")).toBe(7 * 60 + 5);
    expect(parseTime("24:00")).toBeNull();
    expect(parseTime("")).toBeNull();
  });

  it("round-trips a local date and time", () => {
    const at = new Date(2026, 10, 6, 14, 30).getTime();
    expect(dateTimeValue(at)).toBe("2026-11-06T14:30");
    expect(parseDateTime("2026-11-06T14:30")).toBe(at);
    expect(parseDateTime("tomorrow")).toBeNull();
  });

  it("suggests the next full hour", () => {
    const at = new Date(2026, 10, 6, 14, 30, 12).getTime();
    expect(nextHour(at)).toBe(new Date(2026, 10, 6, 15, 0).getTime());
  });
});
