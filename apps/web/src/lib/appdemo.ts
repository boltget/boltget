// The hero app preview on Home (components/home/AppDemo.astro), as data: one
// simulated download (sim.ts) at a moment of the loop, turned into the text,
// classes and paths the card shows. The component renders the finished state
// from this at build time; its script renders every later frame from the same
// functions, so the still in the HTML and the animated card cannot disagree.
import { DEMO_SIM, DownloadSim, type SimOptions } from "./sim";

export const SIM: SimOptions = DEMO_SIM;

function finished(): DownloadSim {
  const s = new DownloadSim(SIM);
  while (!s.finished) s.step();
  return s;
}

/** The finished run: the still, and the peak connection count. */
export const FINAL = finished();
/** Throughput samples in a whole run (the graph's x scale). */
const SAMPLES = Math.ceil(FINAL.finishedAt * 10) + 2;

/** The copy the frames need (the rest of the card is static markup). */
export interface FrameStrings {
  url: string;
  host: string;
  /** "Connecting to {host}" */
  connecting: string;
  /** "{size} · {host} · resumable" */
  meta: string;
  /** "Done. {size} in {time} s." */
  done: string;
  conn: { connecting: string; receiving: string; idle: string };
}

export type Phase = "idle" | "typing" | "pressed" | "probing" | "downloading" | "done";

const phaseRank: Record<Phase, number> = { idle: 0, typing: 1, pressed: 2, probing: 3, downloading: 4, done: 5 };

interface ConnRow {
  state: "connecting" | "receiving" | "idle";
  rate: number;
  note: string;
}

/** A moment of the loop. */
export interface View {
  phase: Phase;
  /** Characters of the link typed so far. */
  typed: number;
  speed: number;
  received: number;
  eta: number | null;
  active: number;
  elapsed: number;
  history: number[];
  conns: ConnRow[];
  /** The finished job is fading out before the loop restarts. */
  fading: boolean;
}

export function viewOf(sim: DownloadSim, phase: Phase, typed: number, fading = false): View {
  return {
    phase,
    typed,
    speed: sim.speed,
    received: sim.downloaded,
    eta: sim.eta(),
    active: sim.active,
    elapsed: sim.finished ? sim.finishedAt : sim.t,
    history: sim.history.slice(),
    conns: sim.conns.map((c) => ({
      state: sim.finished ? "idle" : c.state,
      rate: c.rate,
      note: sim.t - c.noteAt < 1.4 ? c.note : "",
    })),
    fading,
  };
}

/** The finished download, as the server render and reduced motion show it. */
export const STILL: View = viewOf(FINAL, "done", Number.MAX_SAFE_INTEGER);

/** One connection row of the card. */
export interface ConnFrame {
  off: boolean;
  label: string;
  /** Width of the rate bar, a CSS percentage. */
  width: string;
  rate: string;
}

/** Everything on the card that changes, as it should read. */
export interface Frame {
  activeCount: number;
  completedCount: number;
  /** "Completed" is highlighted (the download just landed). */
  bump: boolean;
  focus: boolean;
  /** The typed part of the link; empty shows the placeholder. */
  typed: string;
  caret: boolean;
  pressed: boolean;
  /** The job card is shown. */
  job: boolean;
  fading: boolean;
  meta: string;
  pct: string;
  speed: string;
  received: string;
  left: string;
  connections: string;
  /** Throughput graph paths, empty before there are two samples. */
  area: string;
  line: string;
  conns: ConnFrame[];
  done: boolean;
  doneText: string;
}

export const fill = (msg: string, p: Record<string, string | number>) =>
  msg.replace(/\{(\w+)\}/g, (_, k: string) => String(p[k] ?? ""));

const rate = (v: number) => v.toFixed(1);

function graph(history: number[]): { area: string; line: string } {
  const W = 320;
  const H = 84;
  const max = 72;
  const pts = history.map((v, i) => [(i / (SAMPLES - 1)) * W, H - (v / max) * (H - 6)] as const);
  const last = pts[pts.length - 1];
  if (pts.length < 2 || !last) return { area: "", line: "" };
  const line = pts.map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)} ${y.toFixed(1)}`).join("");
  return { line, area: `${line}L${last[0].toFixed(1)} ${H}L0 ${H}Z` };
}

export function frameOf(v: View, s: FrameStrings): Frame {
  const atLeast = (p: Phase) => phaseRank[v.phase] >= phaseRank[p];
  const done = v.phase === "done";
  const size = `${SIM.total} MiB`;
  return {
    activeCount: atLeast("probing") && !done ? 1 : 0,
    completedCount: done ? 12 : 11,
    bump: done,
    focus: v.phase === "typing" || v.phase === "idle",
    typed: s.url.slice(0, v.typed),
    caret: v.phase === "idle" || v.phase === "typing",
    pressed: v.phase === "pressed",
    job: atLeast("probing"),
    fading: v.fading,
    meta: v.phase === "probing" ? fill(s.connecting, { host: s.host }) : fill(s.meta, { size, host: s.host }),
    pct: v.phase === "probing" ? "" : `${Math.floor((v.received / SIM.total) * 100)}%`,
    speed: done ? "–" : rate(v.speed),
    received: Math.round(v.received).toString(),
    left: done ? "0" : v.eta === null ? "–" : String(Math.ceil(v.eta)),
    connections: String(done ? FINAL.peakConnections : v.active),
    ...graph(v.history),
    conns: v.conns.map((c) => ({
      off: c.state === "idle",
      label: c.note || s.conn[c.state],
      width: `${Math.min(100, (c.rate / 14) * 100)}%`,
      rate: c.state === "idle" ? "" : rate(c.rate),
    })),
    done,
    doneText: fill(s.done, { size, time: v.elapsed.toFixed(1) }),
  };
}
