import { describe, expect, it } from "vitest";
import { rank, score, wordScore } from "./fuzzy";

const labels = (entries: { label: string }[]) => entries.map((e) => e.label);
const fields = (e: { label: string; synonyms?: string }) => e;

describe("fuzzy matching", () => {
  it("matches runs, word starts and spread-out letters", () => {
    expect(wordScore("pause", "pause all")).toBeGreaterThan(wordScore("all", "pause all") ?? 0);
    // A run at a word's start beats one inside a word.
    expect(wordScore("all", "pause all")).toBeGreaterThan(wordScore("all", "install") ?? 0);
    // Letters in order, spread out, still match, below any run.
    expect(wordScore("pa", "pause all")).toBeGreaterThan(99);
    expect(wordScore("psal", "pause all")).not.toBeNull();
    expect(wordScore("psal", "pause all")).toBeLessThan(100);
    expect(wordScore("xyz", "pause all")).toBeNull();
    expect(wordScore("lp", "pl")).toBeNull();
  });

  it("needs every word, in any order", () => {
    expect(score("speed limit", "Limit speed to 5 MiB/s")).not.toBeNull();
    expect(score("limit 5", "Limit speed to 5 MiB/s")).not.toBeNull();
    expect(score("limit 7", "Limit speed to 5 MiB/s")).toBeNull();
    expect(score("", "anything")).toBe(0);
    expect(score("  ", "anything")).toBe(0);
  });

  it("finds entries by their synonyms", () => {
    expect(score("throttle", "Limit speed to 5 MiB/s")).toBeNull();
    expect(score("throttle", "Limit speed to 5 MiB/s", "throttle bandwidth")).not.toBeNull();
    // The label still wins over a synonym for the same word.
    expect(score("pause", "Pause all")).toBeGreaterThan(score("pause", "Stop", "pause") ?? 0);
  });

  it("ranks best first, shorter labels on ties, then the given order", () => {
    const entries = [
      { label: "Open settings" },
      { label: "Pause all" },
      { label: "Resume all", synonyms: "start everything" },
      { label: "Open downloads folder" },
      { label: "Pause" },
    ];
    expect(labels(rank(entries, "pause", fields))).toEqual(["Pause", "Pause all"]);
    expect(labels(rank(entries, "open", fields))).toEqual([
      "Open settings",
      "Open downloads folder",
    ]);
    expect(labels(rank(entries, "start", fields))).toEqual(["Resume all"]);
    expect(labels(rank(entries, "", fields))).toEqual(labels(entries));
    expect(rank(entries, "", fields, 2)).toHaveLength(2);
    expect(rank(entries, "zzz", fields)).toEqual([]);
  });
});
