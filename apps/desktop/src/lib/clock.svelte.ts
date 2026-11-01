// One shared beat for live digits. Progress ticks arrive every 250 ms, but
// digits repaint at 2 Hz (APP-DESIGN #27): rows read `beat` and sample their
// numbers when it changes, so text does not flicker and only one timer runs.
const BEAT_MS = 500;

class Clock {
  /** Increments every 500 ms while anything reads it. */
  beat = $state(0);
  /** Wall-clock milliseconds at the last beat. */
  now = $state(Date.now());
  /** The current day's start; changes at midnight (for "Earlier today"). */
  day = $state(new Date().toDateString());
  #timer: ReturnType<typeof setInterval> | null = null;

  start(): void {
    if (this.#timer !== null) return;
    this.#timer = setInterval(() => {
      this.beat += 1;
      this.now = Date.now();
      const day = new Date(this.now).toDateString();
      if (day !== this.day) this.day = day;
    }, BEAT_MS);
  }
}

export const clock = new Clock();
