// The web server for the real run (real.spec.ts): files with ranges, a
// one-time link, a file behind a cookie, a redirect to a Content-Disposition
// name, and links that expire (signed-URL style) for the link refresh.
import { createServer, type Server, type ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";

export const MIB = 1024 * 1024;

interface FileSpec {
  size: number;
  type: string;
  /** Content-Disposition file name (none: the URL names it). */
  name?: string;
  /** Served once; later requests get 410 Gone. */
  once?: boolean;
  /** Needs the page's cookie, else 403. */
  cookie?: boolean;
  /** Bytes per 20 ms tick and connection (default 64 KiB, ~3 MiB/s). */
  rate?: number;
}

const FILES: Record<string, FileSpec> = {
  "/files/big.zip": { size: 4 * MIB, type: "application/zip", name: "big.zip" },
  "/files/small.zip": { size: 20 * 1024, type: "application/zip", name: "small.zip" },
  "/files/doc.pdf": { size: 3 * MIB, type: "application/pdf", name: "doc.pdf" },
  "/files/once.zip": { size: 4 * MIB, type: "application/zip", name: "once.zip", once: true },
  "/files/rules.zip": { size: 4 * MIB, type: "application/zip", name: "rules.zip" },
  "/files/private.zip": { size: 4 * MIB, type: "application/zip", name: "private.zip" },
  "/files/locked.zip": { size: 4 * MIB, type: "application/zip", cookie: true },
  "/files/locked2.zip": { size: 4 * MIB, type: "application/zip", cookie: true },
  "/cd/blob": { size: 5 * MIB, type: "application/octet-stream", name: "named-archive.zip" },
};

/** Expiring links: `/exp/<token>/<name>.iso`, 24 MiB, slow. */
const EXP_SIZE = 24 * MIB;
const EXP_RATE = 4 * 1024;

const PAGE = (links: string) => `<!doctype html>
<html><head><meta charset="utf-8"><title>Real run</title></head><body>
${links}
</body></html>`;

/** Deterministic bytes, so a test can check a completed file. */
export function fileByte(i: number): number {
  return (i * 31 + 7) & 0xff;
}

export function expected(size: number): Buffer {
  const buf = Buffer.alloc(size);
  for (let i = 0; i < size; i++) buf[i] = fileByte(i);
  return buf;
}

export interface RealServer {
  origin: string;
  /** Requests per path. */
  count(path: string): number;
  /** Cookie headers seen per path. */
  cookies(path: string): string[];
  /** A new expiring link for `name`, dead `ttl` ms after its first request. */
  expiring(name: string, ttl?: number): string;
  close(): Promise<void>;
}

export async function startRealServer(): Promise<RealServer> {
  const seen: { path: string; cookie: string }[] = [];
  const served = new Set<string>();
  /** token -> dies at (ms), or null until its first request. */
  const tokens = new Map<string, { ttl: number; dies: number | null; open: Set<ServerResponse> }>();
  let next = 0;

  const server: Server = createServer((req, res) => {
    const url = new URL(req.url ?? "/", "http://localhost");
    const path = url.pathname;
    seen.push({ path, cookie: req.headers.cookie ?? "" });

    if (path === "/page.html") {
      const links = (url.searchParams.get("links") ?? "")
        .split(",")
        .filter(Boolean)
        .map((href, i) => `<a id="l${i}" href="${href}">${href}</a><br>`)
        .join("\n");
      res.writeHead(200, {
        "content-type": "text/html; charset=utf-8",
        "set-cookie": "session=real-run; Path=/; SameSite=Lax",
      });
      res.end(PAGE(links));
      return;
    }
    if (path === "/redir/archive") {
      res.writeHead(302, { location: "/cd/blob?id=42" }).end();
      return;
    }

    let spec = FILES[path];
    const exp = /^\/exp\/([^/]+)\/([\w.-]+\.iso)$/.exec(path);
    if (exp) {
      const token = tokens.get(exp[1] ?? "");
      if (!token) {
        res.writeHead(403).end("no such token");
        return;
      }
      if (token.dies === null) {
        token.dies = Date.now() + token.ttl;
        // When it dies, the transfers it started end too, so every
        // connection has to ask again (and is refused).
        if (Number.isFinite(token.ttl))
          setTimeout(() => {
            for (const r of token.open) r.destroy();
          }, token.ttl);
      }
      token.open.add(res);
      res.on("close", () => token.open.delete(res));
      if (Date.now() > token.dies) {
        res.writeHead(403, { "content-type": "text/plain" }).end("link expired");
        return;
      }
      spec = { size: EXP_SIZE, type: "application/octet-stream", rate: EXP_RATE };
    }
    if (!spec) {
      res.writeHead(404).end();
      return;
    }
    if (spec.once && served.has(path)) {
      res.writeHead(410).end();
      return;
    }
    if (spec.cookie && !(req.headers.cookie ?? "").includes("session=real-run")) {
      res.writeHead(403, { "content-type": "text/plain" }).end("cookie required");
      return;
    }
    served.add(path);

    let start = 0;
    let end = spec.size; // exclusive
    let status = 200;
    const headers: Record<string, string | number> = { "content-type": spec.type };
    const range = /^bytes=(\d+)-(\d*)$/.exec(req.headers.range ?? "");
    if (spec.once) {
      headers["accept-ranges"] = "none";
    } else {
      headers["accept-ranges"] = "bytes";
      if (range) {
        start = Number(range[1]);
        end = range[2] ? Math.min(Number(range[2]) + 1, spec.size) : spec.size;
        if (start >= spec.size || start >= end) {
          res.writeHead(416, { "content-range": `bytes */${spec.size}` }).end();
          return;
        }
        status = 206;
        headers["content-range"] = `bytes ${start}-${end - 1}/${spec.size}`;
      }
    }
    headers["content-length"] = end - start;
    if (spec.name) headers["content-disposition"] = `attachment; filename="${spec.name}"`;
    res.writeHead(status, headers);
    if (req.method === "HEAD") {
      res.end();
      return;
    }
    const step = spec.rate ?? 64 * 1024;
    let at = start;
    const pump = () => {
      const timer = setInterval(() => {
        if (res.destroyed) {
          clearInterval(timer);
          return;
        }
        const n = Math.min(step, end - at);
        const buf = Buffer.alloc(n);
        for (let i = 0; i < n; i++) buf[i] = fileByte(at + i);
        at += n;
        if (at >= end) {
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
  const origin = `http://127.0.0.1:${port}`;
  return {
    origin,
    count: (path) => seen.filter((s) => s.path === path).length,
    cookies: (path) => seen.filter((s) => s.path === path).map((s) => s.cookie),
    expiring(name, ttl = Number.POSITIVE_INFINITY) {
      const token = `t${++next}x${Math.random().toString(36).slice(2, 8)}`;
      tokens.set(token, { ttl, dies: null, open: new Set() });
      return `${origin}/exp/${token}/${name}`;
    },
    close: () =>
      new Promise((resolve) => {
        server.closeAllConnections();
        server.close(() => resolve());
      }),
  };
}
