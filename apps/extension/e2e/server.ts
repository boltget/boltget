// A local web server for the end-to-end tests: a page of links, files that
// download slowly enough to be paused, a one-time link, and a log of what the
// browser asked for (so a test can tell whether a download was requested
// again).
import { createServer, type IncomingHttpHeaders, type Server } from "node:http";
import type { AddressInfo } from "node:net";

const MIB = 1024 * 1024;

interface FileSpec {
  size: number;
  type: string;
  /** Served once; later requests get 410 Gone. */
  once?: boolean;
  /** Content-Disposition file name (none: the URL names it). */
  name?: string;
}

const FILES: Record<string, FileSpec> = {
  "/files/big.zip": { size: 4 * MIB, type: "application/zip", name: "big.zip" },
  "/files/small.zip": { size: 20 * 1024, type: "application/zip", name: "small.zip" },
  "/files/once.zip": { size: 4 * MIB, type: "application/zip", name: "once.zip", once: true },
  "/files/doc.pdf": { size: 3 * MIB, type: "application/pdf", name: "doc.pdf" },
  "/get": { size: 3 * MIB, type: "application/octet-stream" },
};

const PAGE = `<!doctype html>
<html><head><meta charset="utf-8"><title>Downloads</title></head><body>
<a id="big" href="/files/big.zip">big</a>
<a id="small" href="/files/small.zip">small</a>
<a id="once" href="/files/once.zip">once</a>
<a id="pdf" href="/files/doc.pdf">pdf</a>
<a id="generic" href="/get?id=7">generic</a>
<a id="blob" download="made.zip">blob</a>
<script>
  const blob = new Blob([new Uint8Array(2 * 1024 * 1024)], { type: "application/zip" });
  document.getElementById("blob").href = URL.createObjectURL(blob);
</script>
</body></html>`;

/** One request the browser made. */
export interface Seen {
  path: string;
  headers: IncomingHttpHeaders;
}

export interface TestServer {
  origin: string;
  seen: Seen[];
  /** How often a path was requested. */
  count(path: string): number;
  close(): Promise<void>;
}

/** Deterministic bytes, so a test can check a completed file. */
export function fileByte(i: number): number {
  return (i * 31 + 7) & 0xff;
}

export async function startServer(): Promise<TestServer> {
  const seen: Seen[] = [];
  const served = new Set<string>();
  const server: Server = createServer((req, res) => {
    const url = new URL(req.url ?? "/", "http://localhost");
    seen.push({ path: url.pathname, headers: req.headers });
    if (url.pathname === "/page.html") {
      res.writeHead(200, {
        "content-type": "text/html; charset=utf-8",
        "set-cookie": "session=e2e-cookie; Path=/; SameSite=Lax",
      });
      res.end(PAGE);
      return;
    }
    const spec = FILES[url.pathname];
    if (!spec) {
      res.writeHead(404).end();
      return;
    }
    if (spec.once && served.has(url.pathname)) {
      res.writeHead(410).end();
      return;
    }
    served.add(url.pathname);
    const headers: Record<string, string | number> = {
      "content-type": spec.type,
      "content-length": spec.size,
      // No ranges: a paused download can only continue on its own connection.
      "accept-ranges": "none",
    };
    if (spec.name) headers["content-disposition"] = `attachment; filename="${spec.name}"`;
    res.writeHead(200, headers);
    // 64 KiB every 20 ms: about 1.3 s for 4 MiB, long enough to be paused.
    // A paused download stops reading; back-pressure waits for it.
    let sent = 0;
    const pump = () => {
      const timer = setInterval(() => {
        if (res.destroyed) {
          clearInterval(timer);
          return;
        }
        const n = Math.min(64 * 1024, spec.size - sent);
        const buf = Buffer.alloc(n);
        for (let i = 0; i < n; i++) buf[i] = fileByte(sent + i);
        sent += n;
        if (sent >= spec.size) {
          clearInterval(timer);
          res.end(buf);
        } else if (!res.write(buf)) {
          clearInterval(timer);
          res.once("drain", pump);
        }
      }, 20);
    };
    pump();
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;
  return {
    origin: `http://127.0.0.1:${port}`,
    seen,
    count: (path) => seen.filter((s) => s.path === path).length,
    close: () =>
      new Promise((resolve) => {
        server.closeAllConnections();
        server.close(() => resolve());
      }),
  };
}
