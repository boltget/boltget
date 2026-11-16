// Resume offers (docs/ENGINE.md, "Link refresh"): the browser handed over a
// download that is one waiting for a new link, and the app asks "Use this
// link to resume …?". The backend holds the browser's request; the page
// keeps the offers waiting for an answer, one dialog at a time.
import { t } from "@boltget/ui/messages";
import type { OfferAnswer, ResumeOffer, UiError } from "./bindings";

/**
 * The offers with `offer` added at the end. A newer capture of the same
 * download replaces the older offer, in its place.
 */
export function queueOffer(queue: readonly ResumeOffer[], offer: ResumeOffer): ResumeOffer[] {
  const i = queue.findIndex((o) => o.id === offer.id);
  if (i < 0) return [...queue, offer];
  return queue.map((o, j) => (j === i ? offer : o));
}

/** The offers without the one for `offer`'s download. */
export function dropOffer(queue: readonly ResumeOffer[], offer: ResumeOffer): ResumeOffer[] {
  return queue.filter((o) => o.id !== offer.id);
}

/** The question an offer asks. */
export function offerTitle(offer: ResumeOffer): string {
  return t("offer.resume.title", { name: offer.name, percent: offer.percent });
}

/**
 * What to tell the user after an answer, or `null` when what happened
 * shows for itself (a new row, the add sheet). `reason` turns an error into
 * its sentence.
 */
export function answerNotice(
  answer: OfferAnswer,
  name: string,
  reason: (error: UiError) => string,
): string | null {
  if (answer.outcome === "resumed") return t("offer.resumed", { name });
  if (!answer.refused) return null;
  const why = reason(answer.refused);
  return answer.outcome === "sheet"
    ? t("offer.refused.sheet", { name, reason: why })
    : t("offer.refused.added", { name, reason: why });
}
