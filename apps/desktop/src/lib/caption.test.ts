import { describe, expect, it } from "vitest";
import { captionRect, sameRect } from "./caption";

describe("captionRect", () => {
  it("measures the maximize button from the window's top-right corner", () => {
    // 1280 px window: close is the last 46 px, maximize the 46 before it.
    const rect = captionRect({ left: 1188, top: 0, width: 46, height: 44 }, 1280);
    expect(rect).toEqual({ right: 46, top: 0, width: 46, height: 44 });
  });

  it("stays the same when only the window width changes", () => {
    const narrow = captionRect({ left: 548, top: 0, width: 46, height: 44 }, 640);
    const wide = captionRect({ left: 1188, top: 0, width: 46, height: 44 }, 1280);
    expect(sameRect(narrow, wide)).toBe(true);
    expect(sameRect(undefined, wide)).toBe(false);
  });
});
