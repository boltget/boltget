// The newest published release, resolved at build time from the GitHub REST
// API for /download (docs/WEBSITE.md, Download page). `/releases/latest`
// skips pre-releases, so this lists releases and takes the first one that is
// not a draft. SHA-256 sums come from the release's SHA256SUMS file (falling
// back to the digest GitHub records per asset).
//
// The site must build without the network: if the API is unreachable,
// rate-limited or returns something unexpected, this logs a warning and
// returns `null`, and the page links to the releases page instead.
// GITHUB_TOKEN is used when set (higher rate limit); BOLTGET_RELEASES_OFFLINE=1
// skips the network entirely (offline builds, testing the fallback).

const REPO = "boltget/boltget";
const API = `https://api.github.com/repos/${REPO}/releases?per_page=10`;
const TIMEOUT_MS = 10_000;

/** The release page used when nothing could be resolved. */
export const RELEASES_URL = `https://github.com/${REPO}/releases`;

export type Platform = "windows-x64" | "linux-x64";

export interface Asset {
  name: string;
  /** Known platform, from the file name; `null` for any other file. */
  platform: Platform | null;
  url: string;
  size: number;
  /** Lowercase hex SHA-256, or `null` if neither source had one. */
  sha256: string | null;
}

export interface Release {
  /** Version without the leading `v`, e.g. `0.1.0`. */
  version: string;
  tag: string;
  /** Publication date, `YYYY-MM-DD` (UTC). */
  date: string;
  prerelease: boolean;
  url: string;
  /** The downloadable files, without SHA256SUMS. */
  assets: Asset[];
  /** The SHA256SUMS file, if the release has one. */
  sums: { name: string; url: string } | null;
}

interface ApiAsset {
  name: string;
  size: number;
  browser_download_url: string;
  digest?: string | null;
}

interface ApiRelease {
  tag_name: string;
  html_url: string;
  draft: boolean;
  prerelease: boolean;
  published_at: string | null;
  assets: ApiAsset[];
}

const PLATFORMS: [RegExp, Platform][] = [
  [/-windows-x64\.zip$/, "windows-x64"],
  [/-linux-x64\.tar\.gz$/, "linux-x64"],
];
const ORDER: (Platform | null)[] = ["windows-x64", "linux-x64", null];

function platformOf(name: string): Platform | null {
  return PLATFORMS.find(([re]) => re.test(name))?.[1] ?? null;
}

/** Parses `sha256sum` output: `<64 hex>  [*]<file name>` per line. */
export function parseSums(text: string): Map<string, string> {
  const sums = new Map<string, string>();
  for (const line of text.split(/\r?\n/)) {
    const m = /^([0-9a-fA-F]{64}) [ *](.+)$/.exec(line.trim());
    if (m?.[1] && m[2]) sums.set(m[2].trim(), m[1].toLowerCase());
  }
  return sums;
}

function headers(): Record<string, string> {
  const h: Record<string, string> = {
    Accept: "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
    "User-Agent": "boltget-web-build",
  };
  const token = process.env.GITHUB_TOKEN;
  if (token) h.Authorization = `Bearer ${token}`;
  return h;
}

async function get(url: string, init: RequestInit = {}): Promise<Response> {
  const res = await fetch(url, { ...init, signal: AbortSignal.timeout(TIMEOUT_MS) });
  if (!res.ok) {
    const left = res.headers.get("x-ratelimit-remaining");
    throw new Error(`${res.status} ${res.statusText} from ${url}${left === "0" ? " (rate limit reached)" : ""}`);
  }
  return res;
}

async function resolve(): Promise<Release | null> {
  if (process.env.BOLTGET_RELEASES_OFFLINE === "1") {
    console.warn("[releases] BOLTGET_RELEASES_OFFLINE=1: /download links to the releases page.");
    return null;
  }
  const list = (await (await get(API, { headers: headers() })).json()) as ApiRelease[];
  const latest = list.find((r) => !r.draft && r.published_at);
  if (!latest) throw new Error("no published release");

  const sumsAsset = latest.assets.find((a) => a.name === "SHA256SUMS");
  let sums = new Map<string, string>();
  if (sumsAsset) {
    try {
      sums = parseSums(await (await get(sumsAsset.browser_download_url)).text());
    } catch (err) {
      console.warn(`[releases] could not read SHA256SUMS, using GitHub's digests: ${String(err)}`);
    }
  }

  const assets = latest.assets
    .filter((a) => a !== sumsAsset)
    .map((a): Asset => {
      const digest = a.digest?.startsWith("sha256:") ? a.digest.slice(7).toLowerCase() : null;
      return {
        name: a.name,
        platform: platformOf(a.name),
        url: a.browser_download_url,
        size: a.size,
        sha256: sums.get(a.name) ?? digest,
      };
    })
    .sort((a, b) => ORDER.indexOf(a.platform) - ORDER.indexOf(b.platform));

  return {
    version: latest.tag_name.replace(/^v/, ""),
    tag: latest.tag_name,
    date: (latest.published_at ?? "").slice(0, 10),
    prerelease: latest.prerelease,
    url: latest.html_url,
    assets,
    sums: sumsAsset ? { name: sumsAsset.name, url: sumsAsset.browser_download_url } : null,
  };
}

let cached: Promise<Release | null> | undefined;

/** The newest published release, or `null` (with a warning) if it could not be resolved. */
export function latestRelease(): Promise<Release | null> {
  cached ??= resolve().catch((err: unknown) => {
    console.warn(`[releases] GitHub API unavailable, /download links to the releases page: ${String(err)}`);
    return null;
  });
  return cached;
}
