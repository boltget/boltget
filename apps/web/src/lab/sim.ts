// A deterministic simulation of one segmented download, shared by the design
// lab demos (/lab/a, /lab/b). It is illustrative, not a measurement: the
// numbers it produces are invented and every demo that shows them says so.
//
// What it models, after docs/ENGINE.md:
// - The download starts with 4 connections, each on its own segment.
// - An adaptive controller adds one connection at a time and keeps going only
//   while the last one made the download faster (here: a shared link cap makes
//   extra connections stop paying off).
// - When a connection finishes its segment, it takes over the second half of
//   the largest remaining segment, so all connections finish together.
// Randomness is seeded and time advances in fixed steps, so every visitor sees
// the same run.

export interface SimSegment {
  /** Index of the connection that owns this segment (0-based, stable). */
  conn: number;
  start: number;
  end: number;
  pos: number;
  done: boolean;
}

export interface SimConnection {
  id: number;
  /** Seconds since the start of the download when it came online. */
  born: number;
  base: number;
  freq: number;
  phase: number;
  /** Current rate in MiB/s (after the link cap). */
  rate: number;
  /** "connecting" for the first 0.3 s, then "receiving", "idle" when done. */
  state: "connecting" | "receiving" | "idle";
  /** Short-lived note, e.g. "took half of #3". */
  note: string;
  noteAt: number;
}

export interface SimEvent {
  t: number;
  kind: "online" | "split" | "steal" | "hold";
  conn: number;
  from?: number;
  at?: number;
}

export interface SimOptions {
  total: number;
  start: number;
  max: number;
  /** Per-connection base rate range, MiB/s. */
  rate: [number, number];
  /** Shared link cap, MiB/s. */
  cap: number;
  /** Seconds between controller decisions. */
  every: number;
  /** A segment with less than twice this left is not split. */
  minSplit: number;
  seed: number;
}

export const STEP = 1 / 120;

export function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export class DownloadSim {
  readonly o: SimOptions;
  t = 0;
  segs: SimSegment[] = [];
  conns: SimConnection[] = [];
  events: SimEvent[] = [];
  /** Aggregate MiB/s, sampled every 0.1 s, lightly smoothed. */
  history: number[] = [];
  speed = 0;
  finished = false;
  finishedAt = 0;
  peakConnections = 0;
  private rand: () => number;
  private nextDecision: number;
  private lastAggregate = 0;
  private holding = false;
  private nextSample = 0;
  private smoothed = 0;

  constructor(o: SimOptions) {
    this.o = o;
    this.rand = mulberry32(o.seed);
    for (let i = 0; i < o.start; i++) {
      this.addConnection();
      this.segs.push({
        conn: i,
        start: (o.total * i) / o.start,
        end: (o.total * (i + 1)) / o.start,
        pos: (o.total * i) / o.start,
        done: false,
      });
    }
    this.nextDecision = o.every;
  }

  get downloaded(): number {
    return this.segs.reduce((n, s) => n + (s.pos - s.start), 0);
  }

  get active(): number {
    return this.segs.filter((s) => !s.done).length;
  }

  private between(lo: number, hi: number) {
    return lo + (hi - lo) * this.rand();
  }

  private addConnection(): SimConnection {
    const c: SimConnection = {
      id: this.conns.length,
      born: this.t,
      base: this.between(this.o.rate[0], this.o.rate[1]),
      freq: this.between(1.2, 2.8),
      phase: this.between(0, Math.PI * 2),
      rate: 0,
      state: "connecting",
      note: "",
      noteAt: 0,
    };
    this.conns.push(c);
    this.events.push({ t: this.t, kind: "online", conn: c.id });
    return c;
  }

  private largest(): SimSegment | undefined {
    let best: SimSegment | undefined;
    for (const s of this.segs) {
      if (!s.done && (!best || s.end - s.pos > best.end - best.pos)) best = s;
    }
    if (!best || best.end - best.pos < this.o.minSplit * 2) return undefined;
    return best;
  }

  /** Gives `conn` the second half of the largest remaining segment. */
  private takeHalf(conn: number, kind: "split" | "steal"): boolean {
    const big = this.largest();
    if (!big) return false;
    const mid = big.pos + (big.end - big.pos) / 2;
    this.segs.push({ conn, start: mid, end: big.end, pos: mid, done: false });
    big.end = mid;
    this.events.push({ t: this.t, kind, conn, from: big.conn, at: mid });
    const c = this.conns[conn];
    if (c) {
      c.note = kind === "steal" ? `took half of #${big.conn + 1}` : `split #${big.conn + 1}`;
      c.noteAt = this.t;
      if (kind === "steal") c.state = "receiving";
    }
    return true;
  }

  private want(c: SimConnection): number {
    const age = this.t - c.born;
    const ramp = Math.min(1, 0.1 + age / 0.45);
    return c.base * (1 + 0.16 * Math.sin(c.freq * this.t + c.phase)) * ramp;
  }

  step(): void {
    if (this.finished) return;
    this.t += STEP;

    // Controller: add a connection while the last one paid off.
    if (!this.holding && this.t >= this.nextDecision) {
      this.nextDecision += this.o.every;
      const now = this.speed;
      const paidOff = this.lastAggregate === 0 || now > this.lastAggregate * 1.06;
      if (paidOff && this.conns.length < this.o.max && this.largest()) {
        this.lastAggregate = now;
        const c = this.addConnection();
        this.takeHalf(c.id, "split");
      } else if (!paidOff) {
        this.holding = true;
        this.events.push({ t: this.t, kind: "hold", conn: this.conns.length - 1 });
      }
    }

    // Rates: what each busy connection wants, scaled down to the link cap.
    const busy = new Set(this.segs.filter((s) => !s.done).map((s) => s.conn));
    let sum = 0;
    for (const c of this.conns) if (busy.has(c.id)) sum += this.want(c);
    // The link itself wobbles a little, as real lines do.
    const cap = this.o.cap * (1 + 0.05 * Math.sin(0.9 * this.t) + 0.035 * Math.sin(2.7 * this.t + 1.3));
    const scale = sum > cap ? cap / sum : 1;
    let aggregate = 0;
    for (const c of this.conns) {
      if (busy.has(c.id)) {
        c.rate = this.want(c) * scale;
        aggregate += c.rate;
        if (c.state === "connecting" && this.t - c.born > 0.3) c.state = "receiving";
      } else {
        c.rate = 0;
        c.state = "idle";
      }
    }

    for (const s of [...this.segs]) {
      if (s.done) continue;
      const c = this.conns[s.conn];
      if (!c || c.state === "connecting") continue;
      s.pos = Math.min(s.end, s.pos + c.rate * STEP);
      if (s.pos >= s.end) {
        s.done = true;
        this.takeHalf(s.conn, "steal");
      }
    }

    this.smoothed += (aggregate - this.smoothed) * 0.08;
    this.speed = this.smoothed;
    this.peakConnections = Math.max(this.peakConnections, busy.size);
    if (this.t >= this.nextSample) {
      this.nextSample += 0.1;
      this.history.push(this.smoothed);
    }
    if (this.segs.every((s) => s.done)) {
      this.finished = true;
      this.finishedAt = this.t;
      this.speed = 0;
    }
  }

  /** Advances by `seconds` in fixed steps. */
  advance(seconds: number): void {
    const n = Math.round(seconds / STEP);
    for (let i = 0; i < n && !this.finished; i++) this.step();
  }

  /** Seconds left at the current speed, or null. */
  eta(): number | null {
    if (this.speed <= 0.5) return null;
    return (this.o.total - this.downloaded) / this.speed;
  }

  /**
   * Coverage of the file range [a, b) in MiB: fraction arrived (0..1) and the
   * connection that wrote most of it (or -1).
   */
  cell(a: number, b: number): { f: number; owner: number; head: boolean } {
    let covered = 0;
    let owner = -1;
    let best = 0;
    let head = false;
    for (const s of this.segs) {
      const got = Math.max(0, Math.min(s.pos, b) - Math.max(s.start, a));
      covered += got;
      if (got > best) {
        best = got;
        owner = s.conn;
      }
      if (!s.done && s.pos >= a && s.pos < b) head = true;
    }
    return { f: Math.min(1, covered / (b - a)), owner, head };
  }
}

/** indicatif's HumanBytes: binary units, two decimals. */
export function humanBytes(mib: number): string {
  const bytes = mib * 1024 * 1024;
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(2)} ${units[i]}`;
}

/** indicatif's HumanDuration, roughly: "4 seconds", "1 minute". */
export function humanDuration(s: number): string {
  const n = Math.max(0, Math.round(s));
  if (n < 60) return `${n} second${n === 1 ? "" : "s"}`;
  const m = Math.round(n / 60);
  return `${m} minute${m === 1 ? "" : "s"}`;
}
