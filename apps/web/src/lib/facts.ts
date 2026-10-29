// Project facts read at build time, shown quietly in the footer: the
// workspace version (root Cargo.toml), the short commit the site was built
// from, and the date of the newest benchmark run (lib/bench.ts).
import { execSync } from "node:child_process";
import { parse as parseToml } from "smol-toml";
import { latestRun } from "./bench";

const cargo = import.meta.glob<string>("../../../../Cargo.toml", { query: "?raw", import: "default", eager: true });

function version(): string {
  const text = Object.values(cargo)[0];
  if (text === undefined) throw new Error("facts: root Cargo.toml not found");
  const parsed = parseToml(text) as { workspace?: { package?: { version?: string } } };
  const v = parsed.workspace?.package?.version;
  if (v === undefined) throw new Error("facts: no [workspace.package] version in Cargo.toml");
  return v;
}

function commit(): string {
  const fromEnv = process.env.GITHUB_SHA ?? process.env.CF_PAGES_COMMIT_SHA ?? process.env.VERCEL_GIT_COMMIT_SHA;
  if (fromEnv) return fromEnv.slice(0, 7);
  try {
    return execSync("git rev-parse --short=7 HEAD", { stdio: ["ignore", "pipe", "ignore"] }).toString().trim();
  } catch {
    return "unknown";
  }
}

export interface Facts {
  version: string;
  commit: string;
  benchDate: string;
}

export const facts: Facts = { version: version(), commit: commit(), benchDate: latestRun().date };
