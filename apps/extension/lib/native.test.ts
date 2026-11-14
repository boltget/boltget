import { DEV_HOST_NAME, HOST_NAME, VERSION, type Envelope } from "@boltget/native-messaging";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  classifyError,
  connectionStatus,
  hostName,
  isReply,
  NATIVE_TIMEOUT_MS,
  NativeClient,
  type NativeResult,
} from "./native";
import { DEFAULT_RULES } from "./rules";

afterEach(() => {
  vi.useRealTimers();
});

describe("hosts", () => {
  it("release builds talk to the installed app, development builds to the dev host", () => {
    expect(hostName("release")).toBe(HOST_NAME);
    expect(hostName("development")).toBe(DEV_HOST_NAME);
  });

  it("waits longer than the host's 8 s start plus 6 s reply", () => {
    expect(NATIVE_TIMEOUT_MS).toBeGreaterThanOrEqual(15_000);
  });
});

describe("NativeClient", () => {
  it("wraps the message in an envelope with the version and an id, to the right host", async () => {
    const sent: [string, Envelope][] = [];
    const client = new NativeClient(async (host, message) => {
      sent.push([host, message]);
      return { v: VERSION, id: message.id, type: "accepted", download: 3, confirm: false };
    }, DEV_HOST_NAME);
    const result = await client.request({ type: "capture", url: "https://e.com/a.zip" });
    expect(result).toMatchObject({ type: "accepted", download: 3 });
    expect(sent).toHaveLength(1);
    const [host, envelope] = sent[0] ?? [];
    expect(host).toBe(DEV_HOST_NAME);
    expect(envelope).toMatchObject({ v: VERSION, type: "capture", url: "https://e.com/a.zip" });
    expect(Number.isInteger(envelope?.id)).toBe(true);
    expect(envelope?.id).toBeGreaterThanOrEqual(0);
    expect(envelope?.id).toBeLessThan(2 ** 32);
  });

  it("gives each message its own id", async () => {
    const ids: (number | null | undefined)[] = [];
    const client = new NativeClient(async (_h, m) => {
      ids.push(m.id);
      return { v: 1, id: m.id, type: "status", active: 0, queued: 0, speed: 0 };
    }, HOST_NAME);
    await client.request({ type: "status" });
    await client.request({ type: "status" });
    expect(ids[0]).not.toBe(ids[1]);
  });

  it("turns a missing host into not_installed", async () => {
    const client = new NativeClient(async () => {
      throw new Error("Specified native messaging host not found.");
    }, HOST_NAME);
    expect(await client.request({ type: "status" })).toMatchObject({
      type: "failed",
      reason: "not_installed",
    });
  });

  it("times out a hung host", async () => {
    vi.useFakeTimers();
    const client = new NativeClient(() => new Promise(() => {}), HOST_NAME, 1000);
    const pending = client.request({ type: "status" });
    await vi.advanceTimersByTimeAsync(1000);
    expect(await pending).toMatchObject({ type: "failed", reason: "timeout" });
  });

  it("refuses replies that are not the protocol's, or for another message", async () => {
    for (const reply of [null, "accepted", { type: "accepted" }, { v: 1, type: "maybe" }]) {
      const client = new NativeClient(async () => reply, HOST_NAME);
      expect(await client.request({ type: "status" })).toMatchObject({
        type: "failed",
        reason: "invalid_reply",
      });
    }
    const other = new NativeClient(
      async (_h, m) => ({
        v: 1,
        id: (m.id ?? 0) + 1,
        type: "accepted",
        download: 1,
        confirm: false,
      }),
      HOST_NAME,
    );
    expect(await other.request({ type: "status" })).toMatchObject({ reason: "invalid_reply" });
  });
});

describe("classifyError", () => {
  it.each([
    ["Specified native messaging host not found.", "not_installed"],
    ["No such native application app.boltget.native_host", "not_installed"],
    ["Access to the specified native messaging host is forbidden.", "forbidden"],
    ["Native host has exited.", "host_exited"],
    ["Error when communicating with the native messaging host.", "host_exited"],
    ["An unexpected error occurred", "other"],
  ])("%s -> %s", (message, reason) => {
    expect(classifyError(message)).toBe(reason);
  });
});

describe("connectionStatus", () => {
  const hello = (over: Partial<Extract<NativeResult, { type: "hello" }>> = {}): NativeResult => ({
    v: 1,
    type: "hello",
    app: "Boltget 0.3.0",
    host: "0.3.0",
    oldest: 1,
    current: 1,
    rules: DEFAULT_RULES,
    ...over,
  });

  it("maps every answer to a state the popup shows", () => {
    expect(connectionStatus(hello())).toEqual({
      state: "connected",
      app: "Boltget 0.3.0",
      host: "0.3.0",
      rules: DEFAULT_RULES,
    });
    expect(connectionStatus(hello({ app: null, rules: null }))).toEqual({
      state: "not_running",
      host: "0.3.0",
    });
    expect(connectionStatus(hello({ oldest: VERSION + 1, current: VERSION + 1 }))).toEqual({
      state: "extension_too_old",
    });
    expect(connectionStatus(hello({ oldest: 0, current: 0 }))).toEqual({ state: "app_too_old" });
    expect(
      connectionStatus({ v: 1, type: "error", reason: "app_too_old", message: "" }).state,
    ).toBe("app_too_old");
    expect(
      connectionStatus({ v: 1, type: "error", reason: "extension_too_old", message: "" }).state,
    ).toBe("extension_too_old");
    expect(
      connectionStatus({ v: 1, type: "unavailable", reason: "mismatch", message: "" }).state,
    ).toBe("mismatch");
    expect(
      connectionStatus({ v: 1, type: "unavailable", reason: "not_installed", message: "" }).state,
    ).toBe("not_installed");
    expect(connectionStatus({ type: "failed", reason: "not_installed", detail: "" }).state).toBe(
      "not_installed",
    );
    expect(connectionStatus({ type: "failed", reason: "timeout", detail: "" })).toEqual({
      state: "unreachable",
      reason: "timeout",
    });
    expect(connectionStatus({ v: 1, type: "error", reason: "invalid", message: "" }).state).toBe(
      "unreachable",
    );
  });

  it("knows a reply from anything else", () => {
    expect(isReply({ v: 1, type: "hello" })).toBe(true);
    expect(isReply({ type: "hello" })).toBe(false);
    expect(isReply(undefined)).toBe(false);
  });
});
