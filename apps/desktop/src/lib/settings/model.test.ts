import { t } from "@boltget/ui/messages";
import { describe, expect, it } from "vitest";
import type { AppSettings, CategoryId } from "../bindings";
import { categoryOf, tableOf } from "../list/category";
import {
  DEFAULT_SETTINGS,
  folderFor,
  formatSpeedLimit,
  isAbsolutePath,
  MIB,
  parseConnections,
  parseExtensions,
  parseFolder,
  parseProxyUrl,
  parseSpeedLimit,
} from "./model";

const name = (id: CategoryId) => id.toUpperCase();

describe("settings fields", () => {
  it("takes 1 to 32 connections", () => {
    expect(parseConnections(" 8 ")).toEqual({ ok: true, value: 8 });
    for (const bad of ["0", "33", "", "2.5", "x", "-1"]) {
      expect(parseConnections(bad)).toEqual({
        ok: false,
        problem: t("settings.problem.connections"),
      });
    }
  });

  it("reads speed limits in MiB/s and shows them back", () => {
    expect(parseSpeedLimit("")).toEqual({ ok: true, value: null });
    expect(parseSpeedLimit("2,5")).toEqual({ ok: true, value: 2.5 * MIB });
    expect(parseSpeedLimit(".5")).toEqual({ ok: true, value: 0.5 * MIB });
    expect(parseSpeedLimit("0").ok).toBe(false);
    expect(parseSpeedLimit("fast").ok).toBe(false);
    expect(formatSpeedLimit(null)).toBe("");
    expect(formatSpeedLimit(2.5 * MIB)).toBe("2.5");
    expect(formatSpeedLimit(10 * MIB)).toBe("10");
  });

  it("wants full paths for folders", () => {
    expect(parseFolder("  ")).toEqual({ ok: true, value: null });
    expect(parseFolder("D:\\Media")).toEqual({ ok: true, value: "D:\\Media" });
    expect(isAbsolutePath("\\\\nas\\share\\in")).toBe(true);
    expect(isAbsolutePath("C:/x")).toBe(true);
    expect(parseFolder("Media")).toEqual({
      ok: false,
      problem: t("settings.problem.folder_relative"),
    });
    expect(parseFolder("C:relative").ok).toBe(false);
  });

  it("reads file types as people type them", () => {
    const cats = DEFAULT_SETTINGS.categories;
    expect(parseExtensions(".MKV, *.mp4;webm  mkv", "video", cats, name)).toEqual({
      ok: true,
      value: ["mkv", "mp4", "webm"],
    });
    expect(parseExtensions("", "video", cats, name)).toEqual({ ok: true, value: [] });
    expect(parseExtensions("tar.gz", "archives", cats, name)).toEqual({
      ok: false,
      problem: t("settings.problem.extension"),
    });
    // Another category's type is refused, by name.
    expect(parseExtensions("mp4 mp3", "video", cats, name)).toEqual({
      ok: false,
      problem: t("settings.problem.extension_in", { extension: "mp3", category: "MUSIC" }),
    });
    const many = Array.from({ length: 201 }, (_, i) => `e${i}`).join(" ");
    expect(parseExtensions(many, "video", cats, name).ok).toBe(false);
  });

  it("checks proxy addresses and refuses passwords", () => {
    expect(parseProxyUrl(" proxy.corp:8080 ")).toEqual({ ok: true, value: "proxy.corp:8080" });
    expect(parseProxyUrl("socks5://127.0.0.1:1080").ok).toBe(true);
    expect(parseProxyUrl("https://p.corp").ok).toBe(true);
    expect(parseProxyUrl("")).toEqual({ ok: false, problem: t("settings.problem.proxy_empty") });
    expect(parseProxyUrl("ftp://p.corp")).toEqual({
      ok: false,
      problem: t("settings.problem.proxy"),
    });
    expect(parseProxyUrl("http://me:secret@p.corp")).toEqual({
      ok: false,
      problem: t("settings.problem.proxy_credentials"),
    });
  });
});

describe("where downloads go", () => {
  const settings = (patch: Partial<AppSettings> = {}): AppSettings => ({
    ...DEFAULT_SETTINGS,
    ...patch,
  });
  const withFolders = settings({
    downloadFolder: "D:\\Data",
    categories: DEFAULT_SETTINGS.categories.map((c) =>
      c.id === "video"
        ? { ...c, folder: "D:\\Videos" }
        : c.id === "other"
          ? { ...c, folder: "D:\\Misc" }
          : c,
    ),
  });

  it("uses the category folder, then the default, then Windows'", () => {
    expect(folderFor("a.mkv", settings(), "C:\\Users\\me\\Downloads")).toBe(
      "C:\\Users\\me\\Downloads",
    );
    expect(folderFor("a.mkv", withFolders, "C:\\Downloads")).toBe("D:\\Videos");
    expect(folderFor("a.ZIP", withFolders, "C:\\Downloads")).toBe("D:\\Data");
    expect(folderFor("a.xyz", withFolders, "C:\\Downloads")).toBe("D:\\Misc");
    expect(folderFor(null, withFolders, "C:\\Downloads")).toBe("D:\\Misc");
    expect(folderFor("a.zip", settings({ downloadFolder: "E:\\In" }), null)).toBe("E:\\In");
    expect(folderFor("a.zip", settings(), null)).toBeNull();
  });

  it("follows the file types in Settings", () => {
    const custom = DEFAULT_SETTINGS.categories.map((c) =>
      c.id === "video" ? { ...c, extensions: ["ts", "mkv"] } : c,
    );
    const table = tableOf(custom);
    expect(categoryOf("clip.ts", table)).toBe("video");
    expect(categoryOf("clip.mp4", table)).toBe("other");
    expect(categoryOf("song.mp3", table)).toBe("music");
  });
});
