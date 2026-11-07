// Minimal Chrome DevTools Protocol client for the app's WebView2
// (WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222).
//   node cdp.mjs eval "<js>"           evaluate (awaitPromise) and print JSON
//   node cdp.mjs shot <file.png>        capture the page
//   node cdp.mjs file <script.js>       evaluate a file's contents
import fs from "node:fs";

const PORT = process.env.CDP_PORT ?? "9222";

export async function connect() {
  const targets = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
  const page = targets.find((t) => t.type === "page");
  if (!page) throw new Error("no page target");
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((r, j) => {
    ws.onopen = r;
    ws.onerror = j;
  });
  let id = 0;
  const pending = new Map();
  const listeners = [];
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) reject(new Error(JSON.stringify(msg.error)));
      else resolve(msg.result);
    } else if (msg.method) {
      for (const l of listeners) l(msg);
    }
  };
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const n = ++id;
      pending.set(n, { resolve, reject });
      ws.send(JSON.stringify({ id: n, method, params }));
    });
  const evaluate = async (expression) => {
    const r = await send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
    return r.result.value;
  };
  return { send, evaluate, on: (f) => listeners.push(f), close: () => ws.close() };
}

if (import.meta.url.endsWith(process.argv[1].replaceAll("\\", "/").split("/").pop())) {
  const [cmd, arg] = process.argv.slice(2);
  const c = await connect();
  if (cmd === "eval") console.log(JSON.stringify(await c.evaluate(arg), null, 1));
  else if (cmd === "file") console.log(JSON.stringify(await c.evaluate(fs.readFileSync(arg, "utf8")), null, 1));
  else if (cmd === "shot") {
    const r = await c.send("Page.captureScreenshot", { format: "png" });
    fs.writeFileSync(arg, Buffer.from(r.data, "base64"));
    console.log("saved", arg);
  }
  c.close();
}
