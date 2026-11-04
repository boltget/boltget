import { describe, expect, it } from "vitest";
import { cellAt, cellRange, connections, contiguous, coverage, grid } from "./fileMap";

describe("grid", () => {
  it("fills the width with whole-pixel squares, centred", () => {
    const g = grid(320, 8);
    expect(g.cell).toBeGreaterThanOrEqual(8);
    expect(Number.isInteger(g.cell)).toBe(true);
    const used = g.cols * g.cell + (g.cols - 1) * g.gap;
    expect(used).toBeLessThanOrEqual(320);
    expect(g.left).toBe(Math.floor((320 - used) / 2));
    expect(g.height).toBe(8 * g.cell + 7 * g.gap);
  });

  it("hit-tests squares, not gaps or outside", () => {
    const g = grid(110, 2, 9, 2); // 10 columns of 9 px
    expect(g.cols).toBe(10);
    expect(cellAt(g, g.left + 1, 1)).toBe(0);
    expect(cellAt(g, g.left + 11 + 1, 11 + 1)).toBe(11);
    expect(cellAt(g, -5, 1)).toBe(-1);
    expect(cellAt(g, g.left + 1, 100)).toBe(-1);
  });
});

describe("coverage", () => {
  it("fills cells from the received part of each segment", () => {
    const { fill, heads } = coverage(
      [
        { start: 0, end: 50, received: 25, connection: 1 },
        { start: 50, end: 100, received: 50, connection: null },
      ],
      100,
      10,
    );
    expect([...fill]).toEqual([1, 1, 0.5, 0, 0, 1, 1, 1, 1, 1]);
    expect(heads[2]).toBe(2);
    expect([...heads].filter((h) => h > 0)).toHaveLength(1);
  });

  it("marks who owns each range, numbering connections from 1", () => {
    const { owners } = coverage([{ start: 30, end: 60, received: 0, connection: 4 }], 100, 10);
    expect([...owners]).toEqual([0, 0, 0, 5, 5, 5, 0, 0, 0, 0]);
  });

  it("handles an open-ended segment and nothing at all", () => {
    const open = coverage([{ start: 0, end: null, received: 40, connection: 2 }], 100, 4);
    expect(open.fill[0]).toBe(1);
    expect(open.fill[1]).toBeCloseTo(0.6);
    expect(open.fill[2]).toBe(0);
    expect(coverage([], 0, 4).fill.every((f) => f === 0)).toBe(true);
  });

  it("fills in file order without a segment table", () => {
    expect([...contiguous(0.25, 8).fill]).toEqual([1, 1, 0, 0, 0, 0, 0, 0]);
    expect([...contiguous(0.3, 4).fill].map((f) => Math.round(f * 10) / 10)).toEqual([
      1, 0.2, 0, 0,
    ]);
  });

  it("gives each cell its byte range", () => {
    expect(cellRange(0, 4, 100)).toEqual([0, 25]);
    expect(cellRange(3, 4, 100)).toEqual([75, 100]);
  });

  it("lists the connections at work", () => {
    expect(
      connections(
        [
          { start: 0, end: 100, received: 100, connection: null },
          { start: 100, end: 300, received: 50, connection: 3 },
          { start: 300, end: null, received: 10, connection: 1 },
        ],
        400,
      ),
    ).toEqual([
      { id: 2, done: 10, size: 100 },
      { id: 4, done: 50, size: 200 },
    ]);
    expect(connections(null, 100)).toEqual([]);
  });
});
