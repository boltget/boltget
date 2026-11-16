import { describe, expect, it } from "vitest";
import type { ResumeOffer } from "./bindings";
import { answerNotice, dropOffer, offerTitle, queueOffer } from "./offers";

const offer = (offer: number, id: number, name = "disc.iso", percent = 73): ResumeOffer => ({
  offer,
  id,
  name,
  percent,
});

describe("resume offers", () => {
  it("queues offers in order, a newer one for a download in the older one's place", () => {
    let queue = queueOffer([], offer(1, 10));
    queue = queueOffer(queue, offer(2, 11));
    queue = queueOffer(queue, offer(3, 10));
    expect(queue.map((o) => o.offer)).toEqual([3, 2]);
    expect(dropOffer(queue, offer(1, 10)).map((o) => o.offer)).toEqual([2]);
    expect(dropOffer(queue, offer(9, 12))).toHaveLength(2);
  });

  it("asks with the name and how much is done", () => {
    expect(offerTitle(offer(1, 10))).toBe("Use this link to resume disc.iso (73% done)?");
  });

  it("says what became of the download", () => {
    const reason = () => "The new link points to a different file.";
    expect(answerNotice({ outcome: "resumed", refused: null }, "disc.iso", reason)).toBe(
      "Resuming disc.iso with the new link.",
    );
    expect(answerNotice({ outcome: "added", refused: null }, "disc.iso", reason)).toBeNull();
    expect(answerNotice({ outcome: "sheet", refused: null }, "disc.iso", reason)).toBeNull();
    const refused = { key: "error.refresh_rejected", message: null };
    expect(answerNotice({ outcome: "added", refused }, "disc.iso", reason)).toBe(
      "The new link points to a different file. disc.iso was added as a new download.",
    );
    expect(answerNotice({ outcome: "sheet", refused }, "disc.iso", reason)).toBe(
      "The new link points to a different file. Add disc.iso as a new download if you want it.",
    );
  });
});
