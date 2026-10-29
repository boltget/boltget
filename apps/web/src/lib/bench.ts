// Reads the published benchmark runs from bench/results/ at build time.
// Nothing here computes a new measurement: every number shown on the site is a
// value from summary.csv or environment.json, only formatted for display.
// See docs/BENCHMARK.md for the method.
import { parse as parseToml } from "smol-toml";

const summaries = import.meta.glob<string>("../../../../bench/results/*/summary.csv", {
  query: "?raw",
  import: "default",
  eager: true,
});
const environments = import.meta.glob<string>("../../../../bench/results/*/environment.json", {
  query: "?raw",
  import: "default",
  eager: true,
});
const targetLists = import.meta.glob<string>("../../../../bench/results/*/targets.toml", {
  query: "?raw",
  import: "default",
  eager: true,
});

export interface SummaryRow {
  target: string;
  tool: "idm" | "boltget";
  setting: string;
  runs: number;
  failed: number;
  medianS: number;
  q1S: number;
  q3S: number;
  minS: number;
  maxS: number;
  medianMibS: number;
  /** Most simultaneous connections the lab server saw (lab targets only). */
  serverConnections: number | null;
  /** Connections BoltGet reported (BoltGet only). */
  reportedConnections: number | null;
}

export interface Environment {
  date: string;
  os: string;
  cpu: string;
  logical_cpus: number;
  ram_gib: number;
  disks: { FriendlyName: string; MediaType: string; BusType: string; Size: number };
  network: { Name: string; InterfaceDescription: string; LinkSpeed: string };
  idm_version: string;
  idm_registry_version: string;
  boltget_version: string;
  boltget_commit: string;
  runs: number;
  warmup: number;
  targets: string[];
}

export interface Target {
  name: string;
  url: string;
  size: number;
  notes?: string;
}

export interface Run {
  /** Folder name under bench/results/, e.g. `2026-10-28-idm`. */
  id: string;
  /** `YYYY-MM-DD`, from the folder name. */
  date: string;
  environment: Environment;
  rows: SummaryRow[];
  targets: Target[];
}

function stripBom(text: string): string {
  return text.charCodeAt(0) === 0xfeff ? text.slice(1) : text;
}

/** Minimal RFC 4180 parser: quoted fields, doubled quotes, CRLF or LF. */
function parseCsv(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let quoted = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quoted) {
      if (c === '"' && text[i + 1] === '"') {
        field += '"';
        i++;
      } else if (c === '"') {
        quoted = false;
      } else {
        field += c;
      }
    } else if (c === '"') {
      quoted = true;
    } else if (c === ",") {
      row.push(field);
      field = "";
    } else if (c === "\n" || c === "\r") {
      if (c === "\r" && text[i + 1] === "\n") i++;
      row.push(field);
      if (row.some((f) => f !== "")) rows.push(row);
      row = [];
      field = "";
    } else {
      field += c;
    }
  }
  row.push(field);
  if (row.some((f) => f !== "")) rows.push(row);
  return rows;
}

function num(value: string | undefined, what: string): number {
  const n = Number(value);
  if (value === undefined || value === "" || !Number.isFinite(n)) {
    throw new Error(`bench: ${what} is not a number: ${JSON.stringify(value)}`);
  }
  return n;
}

function optNum(value: string | undefined): number | null {
  return value === undefined || value === "" ? null : Number(value);
}

function parseSummary(text: string): SummaryRow[] {
  const [header, ...lines] = parseCsv(stripBom(text));
  if (!header) throw new Error("bench: empty summary.csv");
  const col = (name: string) => {
    const i = header.indexOf(name);
    if (i < 0) throw new Error(`bench: summary.csv has no column ${name}`);
    return i;
  };
  const c = {
    target: col("target"),
    tool: col("tool"),
    setting: col("setting"),
    runs: col("runs"),
    failed: col("failed"),
    median: col("median_s"),
    q1: col("q1_s"),
    q3: col("q3_s"),
    min: col("min_s"),
    max: col("max_s"),
    mib: col("median_mib_s"),
    server: col("max_server_connections"),
    reported: col("max_reported_connections"),
  };
  return lines.map((f) => {
    const tool = f[c.tool];
    if (tool !== "idm" && tool !== "boltget") throw new Error(`bench: unknown tool ${tool}`);
    return {
      target: f[c.target] ?? "",
      tool,
      setting: f[c.setting] ?? "",
      runs: num(f[c.runs], "runs"),
      failed: num(f[c.failed], "failed"),
      medianS: num(f[c.median], "median_s"),
      q1S: num(f[c.q1], "q1_s"),
      q3S: num(f[c.q3], "q3_s"),
      minS: num(f[c.min], "min_s"),
      maxS: num(f[c.max], "max_s"),
      medianMibS: num(f[c.mib], "median_mib_s"),
      serverConnections: optNum(f[c.server]),
      reportedConnections: optNum(f[c.reported]),
    };
  });
}

function folderOf(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 2] ?? "";
}

function loadRuns(): Run[] {
  const runs: Run[] = [];
  for (const [path, csv] of Object.entries(summaries)) {
    const id = folderOf(path);
    const dir = path.slice(0, path.lastIndexOf("/"));
    const env = environments[`${dir}/environment.json`];
    const toml = targetLists[`${dir}/targets.toml`];
    if (env === undefined || toml === undefined) {
      throw new Error(`bench: ${id} needs environment.json and targets.toml next to summary.csv`);
    }
    const date = /^\d{4}-\d{2}-\d{2}/.exec(id)?.[0];
    if (date === undefined) throw new Error(`bench: results folder ${id} does not start with a date`);
    const parsedTargets = parseToml(toml) as { target?: Target[] };
    runs.push({
      id,
      date,
      environment: JSON.parse(stripBom(env)) as Environment,
      rows: parseSummary(csv),
      targets: parsedTargets.target ?? [],
    });
  }
  runs.sort((a, b) => (a.id < b.id ? 1 : -1));
  return runs;
}

/** Every published run, newest first. */
export const runs: Run[] = loadRuns();

/** The newest published run. */
export function latestRun(): Run {
  const run = runs[0];
  if (run === undefined) throw new Error("bench: no results under bench/results/");
  return run;
}

/** The configurations of one target, in the order IDM, BoltGet adaptive, BoltGet fixed. */
export function rowsFor(run: Run, target: string): SummaryRow[] {
  const rank = (r: SummaryRow) => (r.tool === "idm" ? 0 : r.setting === "auto" ? 1 : 2);
  return run.rows.filter((r) => r.target === target).sort((a, b) => rank(a) - rank(b));
}

/** The row for one configuration, or a build error if the run does not have it. */
export function rowOf(run: Run, target: string, tool: SummaryRow["tool"], setting: string): SummaryRow {
  const row = run.rows.find((r) => r.target === target && r.tool === tool && r.setting === setting);
  if (row === undefined) throw new Error(`bench: ${run.id} has no ${tool} ${setting} row for ${target}`);
  return row;
}

/** The targets of a run, in the order of its environment.json. */
export function targetsOf(run: Run): string[] {
  return run.environment.targets.filter((t) => run.rows.some((r) => r.target === t));
}

/**
 * Compares BoltGet's adaptive default with IDM's default the way
 * docs/BENCHMARK.md does: a difference only when every run of one is faster
 * than every run of the other (the min-to-max spreads do not overlap).
 */
export function verdict(run: Run, target: string): "faster" | "slower" | "overlap" {
  const idm = rowOf(run, target, "idm", "default");
  const bolt = rowOf(run, target, "boltget", "auto");
  if (bolt.maxS < idm.minS) return "faster";
  if (idm.maxS < bolt.minS) return "slower";
  return "overlap";
}

/** Formats a value from the data with a fixed number of decimals. */
export function fixed(value: number, digits: number): string {
  return value.toLocaleString("en-US", {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  });
}
