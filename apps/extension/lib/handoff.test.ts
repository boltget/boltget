// The fallback guarantee with mocks: the app missing, hung, rejecting, a
// one-time link, a reply we do not understand, and the browser's download
// changing while it waits. Only `accepted` may cancel the browser's copy.
import { describe, expect, it, vi } from "vitest";
import { giveBack, handOff, type DownloadControl, type DownloadState } from "./handoff";
import { NativeClient, type NativeResult } from "./native";

/** A fake browser download that behaves like the `downloads` API. */
class FakeDownload implements DownloadControl {
  state: DownloadState = {
    state: "in_progress",
    paused: false,
    canResume: true,
    url: "https://e.com/a.zip",
  };
  gone = false;
  calls: string[] = [];
  restarted: string[] = [];
  failPause = false;
  failResume = false;
  /** Something that happens to the download while the app is asked. */
  meanwhile: ((d: FakeDownload) => void) | null = null;

  async pause() {
    this.calls.push("pause");
    if (this.failPause) throw new Error("Download must be in progress");
    this.state = { ...this.state, paused: true };
  }
  async resume() {
    this.calls.push("resume");
    if (this.failResume || !this.state.canResume) throw new Error("cannot resume");
    this.state = { ...this.state, state: "in_progress", paused: false, error: undefined };
  }
  async cancel() {
    this.calls.push("cancel");
    this.state = { ...this.state, state: "interrupted", error: "USER_CANCELED", paused: false };
  }
  async erase() {
    this.calls.push("erase");
    this.gone = true;
  }
  async get() {
    return this.gone ? null : { ...this.state };
  }
  async restart(url: string) {
    this.calls.push("restart");
    this.restarted.push(url);
  }
}

function reply(result: NativeResult, download?: FakeDownload) {
  return async () => {
    download?.meanwhile?.(download);
    return result;
  };
}

const accepted: NativeResult = { v: 1, type: "accepted", download: 7, confirm: false };
const notInstalled: NativeResult = { type: "failed", reason: "not_installed", detail: "" };
const linkRefused: NativeResult = {
  v: 1,
  type: "rejected",
  reason: "link_refused",
  message: "403",
};

describe("handOff", () => {
  it("accepted: pauses, then cancels and erases the browser's copy", async () => {
    const d = new FakeDownload();
    const h = await handOff(1, reply(accepted, d), d);
    expect(h.outcome).toBe("taken");
    expect(d.calls).toEqual(["pause", "cancel", "erase"]);
  });

  it("accepted with the add sheet open (confirm) is still taken", async () => {
    const d = new FakeDownload();
    const h = await handOff(1, reply({ v: 1, type: "accepted", download: null, confirm: true }), d);
    expect(h.outcome).toBe("taken");
  });

  it("app not installed: the browser's download resumes", async () => {
    const d = new FakeDownload();
    const h = await handOff(1, reply(notInstalled), d);
    expect(h.outcome).toBe("kept");
    expect(d.calls).toEqual(["pause", "resume"]);
    expect(d.state.paused).toBe(false);
  });

  it("app hung: after the timeout the browser's download resumes, and a late reply changes nothing", async () => {
    vi.useFakeTimers();
    try {
      let late: ((v: unknown) => void) | undefined;
      const client = new NativeClient(
        () => new Promise((resolve) => (late = resolve)),
        "app.boltget.native_host",
        20_000,
      );
      const d = new FakeDownload();
      const pending = handOff(1, () => client.request({ type: "capture", url: d.state.url }), d);
      await vi.advanceTimersByTimeAsync(20_000);
      const h = await pending;
      expect(h.result).toMatchObject({ type: "failed", reason: "timeout" });
      expect(h.outcome).toBe("kept");
      late?.({ v: 1, type: "accepted", download: 1, confirm: false });
      await vi.advanceTimersByTimeAsync(10);
      expect(d.calls).toEqual(["pause", "resume"]);
    } finally {
      vi.useRealTimers();
    }
  });

  it.each<[string, NativeResult]>([
    ["rejected: link_refused (a one-time link)", linkRefused],
    ["rejected: failed", { v: 1, type: "rejected", reason: "failed", message: "" }],
    ["unavailable: timeout", { v: 1, type: "unavailable", reason: "timeout", message: "" }],
    ["unavailable: busy", { v: 1, type: "unavailable", reason: "busy", message: "" }],
    ["unavailable: mismatch", { v: 1, type: "unavailable", reason: "mismatch", message: "" }],
    ["error: invalid", { v: 1, type: "error", reason: "invalid", message: "" }],
    ["error: app_too_old", { v: 1, type: "error", reason: "app_too_old", message: "" }],
    [
      "a hello where a capture reply belongs",
      {
        v: 1,
        type: "hello",
        app: null,
        host: "0.3.0",
        oldest: 1,
        current: 1,
        rules: null,
      },
    ],
    ["host exited", { type: "failed", reason: "host_exited", detail: "" }],
    ["invalid reply", { type: "failed", reason: "invalid_reply", detail: "" }],
  ])("%s: the browser keeps its download", async (_name, result) => {
    const d = new FakeDownload();
    const h = await handOff(1, reply(result), d);
    expect(h.outcome).toBe("kept");
    expect(d.calls).not.toContain("cancel");
    expect(d.calls).not.toContain("erase");
    expect(d.state).toMatchObject({ state: "in_progress", paused: false });
  });

  it("an ask that throws counts as a failure, not as accepted", async () => {
    const d = new FakeDownload();
    const h = await handOff(
      1,
      async () => {
        throw new Error("boom");
      },
      d,
    );
    expect(h.outcome).toBe("kept");
    expect(d.calls).toEqual(["pause", "resume"]);
  });

  it("a one-time link whose paused connection was dropped is started again", async () => {
    const d = new FakeDownload();
    d.meanwhile = (x) => {
      x.state = {
        ...x.state,
        state: "interrupted",
        paused: false,
        canResume: false,
        error: "NETWORK_FAILED",
      };
    };
    const h = await handOff(1, reply(linkRefused, d), d);
    expect(h.outcome).toBe("restarted");
    expect(d.restarted).toEqual(["https://e.com/a.zip"]);
    expect(d.calls).toEqual(["pause", "restart", "erase"]);
  });

  it("an interrupted download that can resume is resumed, not started again", async () => {
    const d = new FakeDownload();
    d.meanwhile = (x) => {
      x.state = {
        ...x.state,
        state: "interrupted",
        paused: false,
        canResume: true,
        error: "NETWORK_FAILED",
      };
    };
    const h = await handOff(1, reply(notInstalled, d), d);
    expect(h.outcome).toBe("kept");
    expect(d.calls).toEqual(["pause", "resume"]);
  });

  it("a paused download that refuses to resume is started again", async () => {
    const d = new FakeDownload();
    d.failResume = true;
    const h = await handOff(1, reply(notInstalled), d);
    expect(h.outcome).toBe("restarted");
    expect(d.calls).toEqual(["pause", "resume", "cancel", "restart", "erase"]);
  });

  it("a download the user cancelled meanwhile is left alone", async () => {
    const d = new FakeDownload();
    d.meanwhile = (x) => {
      x.state = {
        ...x.state,
        state: "interrupted",
        paused: false,
        canResume: false,
        error: "USER_CANCELED",
      };
    };
    const h = await handOff(1, reply(notInstalled, d), d);
    expect(h.outcome).toBe("gone");
    expect(d.calls).toEqual(["pause"]);
  });

  it("a download the user removed meanwhile is left alone", async () => {
    const d = new FakeDownload();
    d.meanwhile = (x) => {
      x.gone = true;
    };
    expect((await handOff(1, reply(notInstalled, d), d)).outcome).toBe("gone");
  });

  it("when the pause failed, a refused download simply continues", async () => {
    const d = new FakeDownload();
    d.failPause = true;
    const h = await handOff(1, reply(linkRefused), d);
    expect(h.outcome).toBe("kept");
    expect(d.calls).toEqual(["pause"]);
  });

  it("a download the user paused is not resumed by us", async () => {
    const d = new FakeDownload();
    d.state = { ...d.state, paused: true };
    expect(await giveBack(1, false, d)).toBe("kept");
    expect(d.calls).toEqual([]);
  });

  it("accepted after the browser already finished: both copies are kept", async () => {
    const d = new FakeDownload();
    d.failPause = true;
    d.meanwhile = (x) => {
      x.state = { ...x.state, state: "complete" };
    };
    const h = await handOff(1, reply(accepted, d), d);
    expect(h.outcome).toBe("taken_both");
    expect(d.calls).toEqual(["pause"]);
  });

  it("a finished download needs nothing on refusal", async () => {
    const d = new FakeDownload();
    d.meanwhile = (x) => {
      x.state = { ...x.state, state: "complete", paused: false };
    };
    expect((await handOff(1, reply(linkRefused, d), d)).outcome).toBe("kept");
  });
});
