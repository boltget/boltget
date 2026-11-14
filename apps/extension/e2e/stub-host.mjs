// A stand-in for boltget-native-host in the end-to-end tests: speaks the
// browser's native messaging framing (a 32-bit length in native byte order,
// then UTF-8 JSON), logs every message to $BOLTGET_STUB_DIR/log.jsonl and
// answers as $BOLTGET_STUB_DIR/mode.json says:
//
//   { "capture": "accept" | "reject" | "unavailable" | "hang" | "exit" | "garbage",
//     "app": "Boltget 0.3.0" | null, "rules": {...} | null }
//
// The browser passes its own environment to the host, so the tests set
// BOLTGET_STUB_DIR on the browser they launch.
import { appendFileSync, readFileSync } from "node:fs";
import { join } from "node:path";

const dir = process.env.BOLTGET_STUB_DIR;
if (!dir) process.exit(2);

function mode() {
  try {
    return JSON.parse(readFileSync(join(dir, "mode.json"), "utf8"));
  } catch {
    return {};
  }
}

function send(reply) {
  const body = Buffer.from(JSON.stringify(reply), "utf8");
  const head = Buffer.alloc(4);
  head.writeUInt32LE(body.length, 0);
  process.stdout.write(Buffer.concat([head, body]));
}

let downloads = 0;

function answer(message) {
  appendFileSync(join(dir, "log.jsonl"), JSON.stringify({ at: Date.now(), message }) + "\n");
  const m = mode();
  const base = { v: 1, id: message.id ?? null };
  switch (message.type) {
    case "hello":
      return send({
        ...base,
        type: "hello",
        app: m.app === undefined ? "Boltget 0.3.0" : m.app,
        host: "0.3.0",
        oldest: 1,
        current: 1,
        rules: m.rules ?? null,
      });
    case "capture":
      switch (m.capture ?? "accept") {
        case "accept":
          return send({ ...base, type: "accepted", download: ++downloads, confirm: false });
        case "reject":
          return send({ ...base, type: "rejected", reason: "link_refused", message: "stub" });
        case "unavailable":
          return send({ ...base, type: "unavailable", reason: "not_installed", message: "stub" });
        case "garbage":
          return send({ nonsense: true });
        case "exit":
          return process.exit(0);
        case "hang":
          return;
      }
      return;
    case "add_links":
      return send({ ...base, type: "accepted", download: null, confirm: true });
    case "media_page":
      return send({ ...base, type: "rejected", reason: "unsupported", message: "stub" });
    case "status":
      return send({ ...base, type: "status", active: 0, queued: 0, speed: 0 });
    default:
      return send({ ...base, type: "error", reason: "invalid", message: "stub" });
  }
}

let buffer = Buffer.alloc(0);
process.stdin.on("data", (chunk) => {
  buffer = Buffer.concat([buffer, chunk]);
  while (buffer.length >= 4) {
    const length = buffer.readUInt32LE(0);
    if (buffer.length < 4 + length) break;
    const body = buffer.subarray(4, 4 + length).toString("utf8");
    buffer = buffer.subarray(4 + length);
    answer(JSON.parse(body));
  }
});
// A hung host stays alive until the browser closes the pipe.
process.stdin.on("end", () => process.exit(0));
