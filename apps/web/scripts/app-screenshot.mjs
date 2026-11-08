// Captures the Home hero's screenshot of the desktop app (src/assets/app-light.webp)
// from the real UI running in a browser on its simulated engine, at 2x, through
// the Chrome DevTools protocol. Not part of the build; run by hand:
//
//   pnpm --filter @boltget/desktop dev            (in another terminal)
//   node apps/web/scripts/app-screenshot.mjs
//
// It loads `?mock=12&active=5&theme=light`, removes the one failed download
// (the Delete key, as a user would), selects big_buck_bunny so the details pane
// shows the file map, and pauses one download until the undo toast has gone.
// Needs Chrome or Edge; CHROME overrides the path.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const sharp = createRequire(createRequire(import.meta.url).resolve("astro/package.json"))("sharp");
const url = "http://localhost:1420/?mock=12&active=5&theme=light";
const [w, h] = [1440, 860];
const PORT = 9333;
const chrome = process.env.CHROME ?? "C:/Program Files/Google/Chrome/Application/chrome.exe";
const profile = fs.mkdtempSync(path.join(os.tmpdir(), "boltget-shot-"));
const proc = spawn(chrome, ["--headless=new", `--remote-debugging-port=${PORT}`, `--user-data-dir=${profile}`, "--no-first-run", "--hide-scrollbars", "--force-color-profile=srgb", "about:blank"], { stdio: "ignore" });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let targets;
for (let i = 0; i < 50; i++) {
  try {
    targets = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
    if (targets.some((t) => t.type === "page")) break;
  } catch {}
  await sleep(200);
}
const page = targets.find((t) => t.type === "page");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0;
const pending = new Map();
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
  }
};
const send = (method, params = {}) =>
  new Promise((resolve) => {
    const n = ++id;
    pending.set(n, resolve);
    ws.send(JSON.stringify({ id: n, method, params }));
  });
const evaluate = async (expression) => (await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true })).result;

await send("Emulation.setDeviceMetricsOverride", { width: +w, height: +h, deviceScaleFactor: 2, mobile: false });
await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "light" }] });
await send("Page.enable");
await send("Page.navigate", { url });
await sleep(3000);
const act = `
(async () => {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const rowOf = (s) => [...document.querySelectorAll('.row')].find((r) => r.textContent.includes(s));
  const click = (el, dx = 200) => { const b = el.getBoundingClientRect(); const o = { bubbles: true, clientX: b.left + dx, clientY: b.top + 10, button: 0 }; el.dispatchEvent(new PointerEvent('pointerdown', o)); el.dispatchEvent(new MouseEvent('mousedown', o)); el.dispatchEvent(new PointerEvent('pointerup', o)); el.dispatchEvent(new MouseEvent('mouseup', o)); el.dispatchEvent(new MouseEvent('click', o)); };
  click(rowOf('kexp')); await sleep(200);
  (document.activeElement || document.body).dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', bubbles: true }));
  await sleep(200);
  click(rowOf('big_buck_bunny')); await sleep(200);
  const btn = () => { rowOf('Blender').dispatchEvent(new PointerEvent('pointerover', { bubbles: true })); return new Promise((r) => setTimeout(() => r(rowOf('Blender').querySelector('button')), 100)); };
  let b = await btn(); const l1 = b.getAttribute('aria-label'); b.click(); rowOf('ubuntu').dispatchEvent(new PointerEvent('pointerover', { bubbles: true }));
  await sleep(4600);
  b = await btn(); const l2 = b.getAttribute('aria-label'); b.click(); document.querySelector('.row').parentElement.dispatchEvent(new PointerEvent('pointerleave', { bubbles: true }));
  return [l1, l2];
})()
`;
const result = await evaluate(act);
if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
await sleep(2200);
const r = await send("Page.captureScreenshot", { format: "png" });
const out = path.join(here, "..", "src", "assets", "app-light.webp");
await sharp(Buffer.from(r.result.data, "base64")).webp({ lossless: true, effort: 6 }).toFile(out);
console.log("saved", out);
ws.close();
proc.kill();
