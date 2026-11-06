import { describe, expect, it } from "vitest";
import { ROWS, SECTIONS, searchRows, sectionOf } from "./registry";

const ids = (query: string) => searchRows(query).map((r) => r.id);

describe("settings search", () => {
  it("finds rows by label, description and synonyms", () => {
    expect(ids("proxy")).toEqual(["proxy"]);
    expect(ids("throttle")).toEqual(["limit"]);
    expect(ids("dark")).toEqual(["theme"]);
    expect(ids("startup")).toEqual(["autostart"]);
    expect(ids("CLIPBOARD")).toContain("clipboard");
    // Every word must match.
    expect(ids("proxy socks")).toEqual(["proxy"]);
    expect(ids("proxy zebra")).toEqual([]);
  });

  it("finds nothing for an empty query", () => {
    expect(ids("")).toEqual([]);
    expect(ids("   ")).toEqual([]);
  });

  it("puts every row in a section, in section order", () => {
    const order = SECTIONS.map((s) => s.id);
    const positions = ROWS.map((r) => order.indexOf(r.section));
    expect(positions.every((p) => p >= 0)).toBe(true);
    expect([...positions].sort((a, b) => a - b)).toEqual(positions);
    expect(sectionOf("proxy")).toBe("network");
  });
});
