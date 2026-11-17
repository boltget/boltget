// The store policy (docs/BROWSER-INTEGRATION.md, "Store policy constraints"):
// on YouTube's sites the extension offers nothing about media. No detection,
// no badge, no popup entry, no menu item, and no `media_page` is ever sent
// from or about them. Pure and tested: lib/policy.test.ts.
//
// The policy is a build-time flag (`STORE_POLICY` in lib/platform.ts, from
// wxt.config.ts): on in every build, the stores' and the tests' alike, unless
// a build is made with BOLTGET_STORE_POLICY=off for a browser no store sees.
import { hostMatches, hostOf } from "./rules";

/**
 * The sites where the store builds offer no media (pages and the media they
 * load), each with its subdomains (`m.youtube.com`, `music.youtube.com`, ...).
 */
export const MEDIA_EXCLUDED_SITES = ["youtube.com", "youtu.be", "youtube-nocookie.com"] as const;

/**
 * Hosts whose responses are never hints, wherever the page is: the excluded
 * sites, and YouTube's media servers, which an embedded YouTube player on
 * another site loads from.
 */
export const MEDIA_EXCLUDED_HOSTS = [...MEDIA_EXCLUDED_SITES, "googlevideo.com"] as const;

function onSites(url: string | null | undefined, sites: readonly string[]): boolean {
  const host = hostOf(url);
  return host !== null && sites.some((site) => hostMatches(host, site));
}

/**
 * Whether the extension may offer media for a page: an `http(s)` page that,
 * with the policy on, is not on an excluded site.
 */
export function mediaAllowed(pageUrl: string | null | undefined, policy: boolean): boolean {
  if (!pageUrl) return false;
  let protocol: string;
  try {
    protocol = new URL(pageUrl).protocol;
  } catch {
    return false;
  }
  if (protocol !== "http:" && protocol !== "https:") return false;
  return !(policy && onSites(pageUrl, MEDIA_EXCLUDED_SITES));
}

/**
 * Whether a response may be a hint: with the policy on, neither the response
 * nor what asked for it (a frame of an embedded player, say) is on an
 * excluded host.
 */
export function hintAllowed(
  url: string,
  initiators: readonly (string | null | undefined)[],
  policy: boolean,
): boolean {
  if (!policy) return true;
  return (
    !onSites(url, MEDIA_EXCLUDED_HOSTS) && !initiators.some((i) => onSites(i, MEDIA_EXCLUDED_HOSTS))
  );
}
