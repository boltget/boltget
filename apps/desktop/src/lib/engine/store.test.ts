import { describe, expect, it, vi } from "vitest";
import type {
  DownloadEvent,
  DownloadRow,
  DownloadState,
  EngineMessage,
  ProgressItem,
  UiError,
} from "../bindings";
import { errorText } from "./index";
import { DownloadStore, type EngineApi, type Result } from "./store.svelte";

function row(id: number, extra: Partial<DownloadRow> = {}): DownloadRow {
  return {
    id,
    state: "paused",
    url: `https://example.com/${id}.bin`,
    fileName: `${id}.bin`,
    path: null,
    total: 1000,
    downloaded: 0,
    bps: 0,
    connections: 0,
    resumable: true,
    speedLimit: null,
    error: null,
    createdAt: id,
    completedAt: null,
    ...extra,
  };
}

function progress(id: number, downloaded: number, extra: Partial<ProgressItem> = {}): ProgressItem {
  return { id, downloaded, total: 1000, bps: 500, connections: 4, segments: null, ...extra };
}

function batch(parts: Partial<Extract<EngineMessage, { type: "batch" }>>): EngineMessage {
  return { type: "batch", rows: [], removed: [], progress: [], events: [], ...parts };
}

const ok = <T>(data: T): Promise<Result<T>> => Promise.resolve({ status: "ok", data });
const fail = (key: string): Promise<Result<never>> =>
  Promise.resolve({ status: "error", error: { key, message: null } });

/** A fake backend: the test sends messages through `emit`. */
function fakeApi(overrides: Partial<EngineApi["commands"]> = {}) {
  let emit: (message: EngineMessage) => void = () => {};
  const commands: EngineApi["commands"] = {
    addDownload: vi.fn((req) => ok(row(99, { url: req.url, state: "queued", fileName: null }))),
    probeUrl: vi.fn(() =>
      ok({
        finalUrl: "https://example.com/f.iso",
        fileName: "f.iso",
        size: 10,
        resumable: true,
        contentType: "application/octet-stream",
        webPage: false,
        redirects: 0,
      }),
    ),
    pauseDownload: vi.fn(() => ok("paused" as const)),
    resumeDownload: vi.fn(() => ok(null)),
    removeDownload: vi.fn(() => ok(null)),
    refreshLink: vi.fn(() => ok(null)),
    setSpeedLimit: vi.fn(() => ok(null)),
    updateSettings: vi.fn((s) => ok({ speedLimit: s.speedLimit ?? null })),
    setFocusedDownload: vi.fn(() => Promise.resolve()),
    setUiVisible: vi.fn(() => Promise.resolve()),
    downloadDetails: vi.fn((id) => ok({ id, partialPath: null, checksum: null, sources: [] })),
    openDownload: vi.fn(() => ok(null)),
    showDownloadInFolder: vi.fn(() => ok(null)),
    ...overrides,
  };
  const api: EngineApi = {
    subscribe: vi.fn(async (onMessage) => {
      emit = onMessage;
    }),
    commands,
  };
  return { api, commands, emit: (m: EngineMessage) => emit(m) };
}

async function connected(overrides: Partial<EngineApi["commands"]> = {}, rows = [row(1), row(2)]) {
  const fake = fakeApi(overrides);
  const store = new DownloadStore(fake.api);
  await store.connect();
  fake.emit({
    type: "snapshot",
    status: { state: "ready" },
    downloads: rows,
    settings: { speedLimit: null },
  });
  return { store, ...fake };
}

describe("DownloadStore", () => {
  it("loads the snapshot", async () => {
    const { store, api } = await connected();
    expect(api.subscribe).toHaveBeenCalledOnce();
    expect(store.status).toEqual({ state: "ready" });
    expect(store.ids).toEqual([1, 2]);
    expect(store.downloads.map((d) => d.fileName)).toEqual(["1.bin", "2.bin"]);
    await store.connect();
    expect(api.subscribe).toHaveBeenCalledOnce();
  });

  it("applies progress in place without replacing the list", async () => {
    const { store, emit } = await connected();
    const ids = store.ids;
    const first = store.get(1);
    emit(
      batch({
        progress: [
          progress(1, 400, { segments: [{ start: 0, end: 1000, received: 400, connection: 0 }] }),
        ],
      }),
    );
    expect(store.ids).toBe(ids);
    expect(store.get(1)).toBe(first);
    expect(first?.downloaded).toBe(400);
    expect(first?.bps).toBe(500);
    expect(first?.connections).toBe(4);
    expect(first?.segments).toHaveLength(1);
    expect(store.get(2)?.downloaded).toBe(0);
  });

  it("keeps a large list's identity across ticks", async () => {
    const rows = Array.from({ length: 500 }, (_, i) => row(i + 1));
    const { store, emit } = await connected({}, rows);
    const ids = store.ids;
    for (let tick = 1; tick <= 40; tick++) {
      emit(
        batch({
          progress: Array.from({ length: 20 }, (_, i) => progress(i + 1, tick * 10)),
        }),
      );
    }
    expect(store.ids).toBe(ids);
    expect(store.get(20)?.downloaded).toBe(400);
    expect(store.get(21)?.downloaded).toBe(0);
  });

  it("adds, updates and removes rows from batches, and reports events", async () => {
    const { store, emit } = await connected();
    const seen: DownloadEvent[] = [];
    const off = store.onEvent((e) => seen.push(e));
    emit(
      batch({
        rows: [row(3, { state: "downloading" }), row(1, { state: "completed", downloaded: 1000 })],
        removed: [2],
        events: [
          { kind: "added", id: 3 },
          { kind: "completed", id: 1, path: "C:\\d\\1.bin" },
          { kind: "removed", id: 2 },
        ],
      }),
    );
    expect(store.ids).toEqual([1, 3]);
    expect(store.get(1)?.state).toBe("completed");
    expect(store.get(3)?.state).toBe("downloading");
    expect(seen.map((e) => e.kind)).toEqual(["added", "completed", "removed"]);
    off();
    emit(batch({ events: [{ kind: "added", id: 4 }] }));
    expect(seen).toHaveLength(3);
  });

  it("replaces everything on a later snapshot, keeping surviving rows", async () => {
    const { store, emit } = await connected();
    const one = store.get(1);
    emit({
      type: "snapshot",
      status: { state: "ready" },
      downloads: [row(1, { downloaded: 500 }), row(5)],
      settings: { speedLimit: 1000 },
    });
    expect(store.ids).toEqual([1, 5]);
    expect(store.get(1)).toBe(one);
    expect(one?.downloaded).toBe(500);
    expect(store.settings.speedLimit).toBe(1000);
  });

  it("follows status and settings messages", async () => {
    const { store, emit } = await connected();
    emit({ type: "status", status: { state: "locked", dir: "C:\\Boltget" } });
    expect(store.status).toEqual({ state: "locked", dir: "C:\\Boltget" });
    emit({ type: "settings", settings: { speedLimit: 42 } });
    expect(store.settings.speedLimit).toBe(42);
  });

  it("adds a download and shows its row at once", async () => {
    const { store, commands } = await connected();
    const result = await store.add({ url: "https://example.com/new.iso" });
    expect(result.status).toBe("ok");
    expect(commands.addDownload).toHaveBeenCalledWith({ url: "https://example.com/new.iso" });
    expect(store.ids).toEqual([1, 2, 99]);
    expect(store.get(99)?.state).toBe("queued");
  });

  it("pauses optimistically and settles on the backend's answer", async () => {
    let resolve: (r: Result<DownloadState>) => void = () => {};
    const { store } = await connected(
      {
        pauseDownload: vi.fn(() => new Promise<Result<DownloadState>>((r) => (resolve = r))),
      },
      [row(1, { state: "downloading", bps: 900, connections: 8 })],
    );
    const pending = store.pause(1);
    const d = store.get(1);
    expect(d?.state).toBe("paused");
    expect(d?.bps).toBe(0);
    resolve({ status: "ok", data: "failed" });
    await pending;
    expect(d?.state).toBe("failed");
  });

  it("rolls a failed pause back and records the error", async () => {
    const { store } = await connected({ pauseDownload: vi.fn(() => fail("error.invalid_state")) }, [
      row(1, { state: "downloading", bps: 900, connections: 8 }),
    ]);
    const result = await store.pause(1);
    expect(result.status).toBe("error");
    const d = store.get(1);
    expect(d?.state).toBe("downloading");
    expect(d?.bps).toBe(900);
    expect(d?.connections).toBe(8);
    expect(store.lastError?.key).toBe("error.invalid_state");
  });

  it("resumes optimistically and rolls back on failure", async () => {
    const error = { kind: "permanent", key: "error.http.not_found", message: "404" } as const;
    const { store, commands } = await connected(
      { resumeDownload: vi.fn(() => fail("error.engine_unavailable")) },
      [row(1, { state: "failed", error })],
    );
    const pending = store.resume(1, true);
    expect(store.get(1)?.state).toBe("queued");
    expect(store.get(1)?.error).toBeNull();
    await pending;
    expect(commands.resumeDownload).toHaveBeenCalledWith(1, true);
    expect(store.get(1)?.state).toBe("failed");
    expect(store.get(1)?.error).toEqual(error);
  });

  it("removes optimistically and restores the row if removing failed", async () => {
    const { store, commands } = await connected({
      removeDownload: vi
        .fn<EngineApi["commands"]["removeDownload"]>()
        .mockImplementationOnce(() => ok(null))
        .mockImplementationOnce(() => fail("error.disk")),
    });
    await store.remove(1, true);
    expect(commands.removeDownload).toHaveBeenCalledWith(1, true);
    expect(store.ids).toEqual([2]);
    const two = store.get(2);
    const pending = store.remove(2);
    expect(store.ids).toEqual([]);
    await pending;
    expect(store.ids).toEqual([2]);
    expect(store.get(2)).toBe(two);
  });

  it("changes speed limits optimistically", async () => {
    const { store, commands } = await connected({
      setSpeedLimit: vi
        .fn<EngineApi["commands"]["setSpeedLimit"]>()
        .mockImplementationOnce(() => ok(null))
        .mockImplementationOnce(() => fail("error.database")),
    });
    await store.setSpeedLimit({ kind: "download", id: 1 }, 1_000_000);
    expect(store.get(1)?.speedLimit).toBe(1_000_000);
    await store.setSpeedLimit({ kind: "global" }, 500);
    expect(store.settings.speedLimit).toBeNull();
    expect(commands.setSpeedLimit).toHaveBeenCalledTimes(2);
  });

  it("streams the focused download's map and clears the previous one", async () => {
    const { store, emit, commands } = await connected();
    await store.focus(1);
    emit(batch({ progress: [progress(1, 1, { segments: [] })] }));
    expect(store.get(1)?.segments).toEqual([]);
    await store.focus(2);
    expect(store.get(1)?.segments).toBeNull();
    expect(commands.setFocusedDownload).toHaveBeenLastCalledWith(2);
    await store.focus(2);
    expect(commands.setFocusedDownload).toHaveBeenCalledTimes(2);
  });

  it("passes probe results and visibility through", async () => {
    const { store, commands } = await connected();
    const probe = await store.probe("https://example.com/f.iso");
    expect(probe.status === "ok" && probe.data.fileName).toBe("f.iso");
    await store.setVisible(false);
    expect(commands.setUiVisible).toHaveBeenCalledWith(false);
  });
});

describe("errorText", () => {
  it("uses the catalog, with a fallback for unknown keys", () => {
    const known: UiError = { key: "error.network", message: "reset" };
    expect(errorText(known)).toBe("The connection was lost.");
    expect(errorText({ key: "error.from_a_newer_engine" })).toBe(
      "Something went wrong. Try again.",
    );
  });
});
