// Fuzzy matching for the command palette (APP-DESIGN #20). Every word of the
// query must match the label, or the entry's synonyms ("throttle" finds
// "Limit speed"), in any order. Within a word, a run of letters at a word's
// start counts most, then a run anywhere, then letters spread out in order
// ("pa" finds "Pause all"). Higher scores are better; `null` is no match.

/** Whether position `i` of `text` starts a word. */
function boundary(text: string, i: number): boolean {
  if (i === 0) return true;
  const before = text[i - 1] ?? "";
  return !/[a-z0-9]/i.test(before);
}

/** How well one query word matches `text` (both lowercase). */
export function wordScore(word: string, text: string): number | null {
  if (!word) return 0;
  // A run of the word's letters: best at the start of a word.
  let best: number | null = null;
  for (let at = text.indexOf(word); at >= 0; at = text.indexOf(word, at + 1)) {
    const score = 100 + (boundary(text, at) ? 60 : 0) + (at === 0 ? 20 : 0) - Math.min(at, 40) / 4;
    if (best === null || score > best) best = score;
  }
  if (best !== null) return best;
  // The letters in order, spread out: each one scores more next to the
  // previous one or at a word's start.
  let score = 0;
  let from = 0;
  let previous = -2;
  for (const ch of word) {
    const i = text.indexOf(ch, from);
    if (i < 0) return null;
    score += 1 + (i === previous + 1 ? 6 : 0) + (boundary(text, i) ? 8 : 0);
    previous = i;
    from = i + 1;
  }
  // Spread-out matches always rank below a run of the same word.
  return Math.min(score, 99);
}

/** Synonyms match too, a little below the label. */
const SYNONYM_WEIGHT = 0.8;

/**
 * How well `query` matches an entry's `label` (and its `synonyms`), or
 * `null` when some word of the query matches neither.
 */
export function score(query: string, label: string, synonyms = ""): number | null {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return 0;
  const l = label.toLowerCase();
  const s = synonyms.toLowerCase();
  let total = 0;
  for (const word of words) {
    const inLabel = wordScore(word, l);
    const inSynonyms = s ? wordScore(word, s) : null;
    const best = Math.max(inLabel ?? -1, inSynonyms === null ? -1 : inSynonyms * SYNONYM_WEIGHT);
    if (best < 0) return null;
    total += best;
  }
  return total;
}

/**
 * Entries that match `query`, best first; ties keep the shorter label, then
 * the given order. An empty query keeps everything in order.
 */
export function rank<T>(
  entries: readonly T[],
  query: string,
  fields: (entry: T) => { label: string; synonyms?: string },
  limit = Infinity,
): T[] {
  if (!query.trim()) return entries.slice(0, limit);
  const scored: { entry: T; score: number; length: number; index: number }[] = [];
  entries.forEach((entry, index) => {
    const { label, synonyms } = fields(entry);
    const s = score(query, label, synonyms);
    if (s !== null) scored.push({ entry, score: s, length: label.length, index });
  });
  scored.sort((a, b) => b.score - a.score || a.length - b.length || a.index - b.index);
  return scored.slice(0, limit).map((s) => s.entry);
}
