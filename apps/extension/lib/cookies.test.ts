import { describe, expect, it } from "vitest";
import {
  collectCookies,
  MAX_COOKIES,
  toRecord,
  topLevelSite,
  type BrowserCookie,
  type CookieQuery,
} from "./cookies";

function cookie(over: Partial<BrowserCookie> = {}): BrowserCookie {
  return {
    name: "sid",
    value: "abc",
    domain: ".example.com",
    path: "/",
    hostOnly: false,
    secure: true,
    httpOnly: true,
    sameSite: "lax",
    session: false,
    expirationDate: 1_900_000_000.75,
    ...over,
  };
}

describe("toRecord", () => {
  it("maps the browser's fields to the protocol's", () => {
    expect(toRecord(cookie())).toEqual({
      name: "sid",
      value: "abc",
      domain: ".example.com",
      path: "/",
      host_only: false,
      secure: true,
      http_only: true,
      same_site: "lax",
      expires: 1_900_000_000,
    });
  });

  it("leaves out the expiry of a session cookie and an unknown sameSite", () => {
    const record = toRecord(cookie({ session: true, sameSite: "weird" }));
    expect(record?.expires).toBeUndefined();
    expect(record?.same_site).toBeUndefined();
  });

  it("refuses what the host would refuse", () => {
    expect(toRecord(cookie({ value: "a;b" }))).toBeNull();
    expect(toRecord(cookie({ name: "a\nb" }))).toBeNull();
    expect(toRecord(cookie({ domain: "" }))).toBeNull();
    expect(toRecord(cookie({ domain: "exa mple.com" }))).toBeNull();
  });

  it("fixes a path that does not start with a slash", () => {
    expect(toRecord(cookie({ path: "" }))?.path).toBe("/");
  });
});

describe("collectCookies", () => {
  it("queries each URL, unpartitioned and in the page's partition, and merges", async () => {
    const queries: CookieQuery[] = [];
    const getAll = async (q: CookieQuery) => {
      queries.push(q);
      if (q.partitionKey) return [cookie({ name: "chips", partitionKey: q.partitionKey })];
      return [cookie({ name: q.url.includes("cdn") ? "cdn" : "sid" })];
    };
    const records = await collectCookies(
      getAll,
      ["https://e.com/get", "https://cdn.e.com/a.zip", "https://e.com/get", null],
      { storeId: "0", pageUrl: "https://www.e.com/page" },
    );
    expect(queries).toEqual([
      { url: "https://e.com/get", storeId: "0" },
      {
        url: "https://e.com/get",
        storeId: "0",
        partitionKey: { topLevelSite: "https://www.e.com" },
      },
      { url: "https://cdn.e.com/a.zip", storeId: "0" },
      {
        url: "https://cdn.e.com/a.zip",
        storeId: "0",
        partitionKey: { topLevelSite: "https://www.e.com" },
      },
    ]);
    expect(records.map((r) => r.name)).toEqual(["sid", "chips", "cdn"]);
  });

  it("asks across first-party isolation on Firefox", async () => {
    const queries: CookieQuery[] = [];
    await collectCookies(
      async (q) => {
        queries.push(q);
        return [];
      },
      ["https://e.com/a"],
      { firefox: true, storeId: "firefox-private" },
    );
    expect(queries).toEqual([
      { url: "https://e.com/a", storeId: "firefox-private", firstPartyDomain: null },
    ]);
  });

  it("gives what it can when a query fails (no site access, an older browser)", async () => {
    const records = await collectCookies(
      async (q) => {
        if (q.partitionKey) throw new Error("Unexpected property partitionKey");
        return [cookie()];
      },
      ["https://e.com/a"],
      { pageUrl: "https://e.com/" },
    );
    expect(records).toHaveLength(1);
    expect(
      await collectCookies(async () => {
        throw new Error("no host permission");
      }, ["https://e.com/a"]),
    ).toEqual([]);
  });

  it("stays within the protocol's 300 cookies and 64 KiB", async () => {
    const many = Array.from({ length: 400 }, (_, i) => cookie({ name: `c${i}`, value: "v" }));
    expect(await collectCookies(async () => many, ["https://e.com/"])).toHaveLength(MAX_COOKIES);

    const big = Array.from({ length: 10 }, (_, i) =>
      cookie({ name: `c${i}`, value: "x".repeat(10_000) }),
    );
    const records = await collectCookies(async () => big, ["https://e.com/"]);
    expect(records).toHaveLength(6);
  });
});

describe("topLevelSite", () => {
  it("is the page's scheme and host", () => {
    expect(topLevelSite("https://www.e.com/a/b?c")).toBe("https://www.e.com");
    expect(topLevelSite("chrome://newtab")).toBe(null);
    expect(topLevelSite(null)).toBe(null);
    expect(topLevelSite("junk")).toBe(null);
  });
});
