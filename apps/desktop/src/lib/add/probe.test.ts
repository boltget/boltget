import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ProbePreview } from "../bindings";
import type { Result } from "../engine";
import { LIST_CONCURRENCY, ProbeQueue, problemOf } from "./probe";

function preview(url: string): ProbePreview {
  return {
    finalUrl: url,
    fileName: url.split("/").pop() ?? "f",
    size: 1,
    resumable: true,
    contentType: null,
    webPage: false,
    redirects: 0,
    checksum: null,
  };
}

/** A backend whose probes answer only when the test says so. */
function backend() {
  const pending = new Map<string, (r: Result<ProbePreview>) => void>();
  const probe = vi.fn(
    (url: string) =>
      new Promise<Result<ProbePreview>>((resolve) => {
        pending.set(url, resolve);
      }),
  );
  async function answer(url: string, ok = true) {
    const resolve = pending.get(url);
    if (!resolve) throw new Error(`no probe for ${url}`);
    pending.delete(url);
    resolve(
      ok
        ? { status: "ok", data: preview(url) }
        : { status: "error", error: { key: "error.network", message: null } },
    );
    await vi.advanceTimersByTimeAsync(0);
  }
  return { probe, answer, pending };
}

describe("probe queue", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("waits for typing to pause before probing", async () => {
    const b = backend();
    const q = new ProbeQueue({ probe: b.probe, onChange: () => {}, debounceMs: 300 });
    q.want(["https://a.org/x"]);
    await vi.advanceTimersByTimeAsync(200);
    q.want(["https://a.org/xy"]);
    await vi.advanceTimersByTimeAsync(299);
    expect(b.probe).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(b.probe).toHaveBeenCalledExactlyOnceWith("https://a.org/xy");
  });

  it("starts at once for a paste", () => {
    const b = backend();
    const q = new ProbeQueue({ probe: b.probe, onChange: () => {} });
    q.want(["https://a.org/x"], true);
    expect(b.probe).toHaveBeenCalledOnce();
    expect(q.get("https://a.org/x")?.status).toBe("probing");
  });

  it("keeps one probe in flight and skips links that left the field", async () => {
    const b = backend();
    const q = new ProbeQueue({ probe: b.probe, onChange: () => {} });
    q.want(["https://a.org/1"], true);
    q.want(["https://a.org/2"], true);
    q.want(["https://a.org/3"], true);
    expect(b.probe).toHaveBeenCalledTimes(1);
    expect(q.running).toBe(1);
    await b.answer("https://a.org/1");
    // 2 was replaced by 3 before its turn came: only 3 is probed.
    expect(b.probe).toHaveBeenCalledTimes(2);
    expect(b.probe).toHaveBeenLastCalledWith("https://a.org/3");
    // The stale answer is kept, not shown: the field asks only for 3.
    expect(q.get("https://a.org/1")?.status).toBe("ready");
  });

  it("probes each link once", async () => {
    const b = backend();
    const q = new ProbeQueue({ probe: b.probe, onChange: () => {} });
    q.want(["https://a.org/1"], true);
    await b.answer("https://a.org/1");
    q.want(["https://a.org/1"], true);
    expect(b.probe).toHaveBeenCalledTimes(1);
  });

  it("probes a few links of a list at once", async () => {
    const b = backend();
    const q = new ProbeQueue({ probe: b.probe, onChange: () => {} });
    const urls = Array.from({ length: 5 }, (_, i) => `https://a.org/${i}`);
    q.want(urls, true);
    expect(b.probe).toHaveBeenCalledTimes(LIST_CONCURRENCY);
    await b.answer("https://a.org/0");
    expect(b.probe).toHaveBeenCalledTimes(LIST_CONCURRENCY + 1);
  });

  it("records failures and probes again on retry", async () => {
    const b = backend();
    const q = new ProbeQueue({ probe: b.probe, onChange: () => {} });
    q.want(["https://a.org/x"], true);
    await b.answer("https://a.org/x", false);
    const entry = q.get("https://a.org/x");
    expect(entry?.status).toBe("failed");
    q.retry("https://a.org/x");
    expect(b.probe).toHaveBeenCalledTimes(2);
  });

  it("ignores answers after it is disposed", async () => {
    const b = backend();
    const onChange = vi.fn();
    const q = new ProbeQueue({ probe: b.probe, onChange });
    q.want(["https://a.org/x"], true);
    q.want(["https://a.org/y"]);
    q.dispose();
    onChange.mockClear();
    await b.answer("https://a.org/x");
    await vi.advanceTimersByTimeAsync(1000);
    expect(onChange).not.toHaveBeenCalled();
    expect(b.probe).toHaveBeenCalledTimes(1);
  });

  it("words network failures as unreachable and refusals as sign-in", () => {
    expect(problemOf({ key: "error.timeout" })).toBe("unreachable");
    expect(problemOf({ key: "error.http.forbidden" })).toBe("auth");
    expect(problemOf({ key: "error.http.not_found" })).toBe("other");
  });
});
