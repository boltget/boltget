// Fills the running app with the 500 downloads of docs/APP-PERFORMANCE.md:
// 200 small files that complete at once and 300 large, throttled ones in the
// Main queue, set to run 20 at once. Needs boltget-testserver on port 8090
// and the app started with the DevTools port (see README.md).
//   node seed.mjs <scratch download folder>
import { connect } from "./cdp.mjs";

const dir = process.argv[2];
if (!dir) throw new Error("usage: node seed.mjs <scratch download folder>");
const c = await connect();
const result = await c.evaluate(`(async () => {
  const inv = window.__TAURI_INTERNALS__.invoke;
  const dir = ${JSON.stringify(dir)};
  const base = "http://127.0.0.1:8090/data";
  await inv("update_queue", { id: 1, patch: { maxConcurrent: 20 } });
  for (let i = 0; i < 200; i++) {
    await inv("add_download", { request: { url: base + "?size=65536&seed=" + i + "&cd=attachment%3B%20filename%3Dhistory-" + i + ".bin", directory: dir } });
  }
  for (let i = 0; i < 300; i++) {
    await inv("add_download", { request: { url: base + "?size=2147483648&seed=" + (1000 + i) + "&throttle=131072&cd=attachment%3B%20filename%3Dlarge-" + i + ".iso", directory: dir, connections: 2 } });
  }
  return "added 500 downloads";
})()`);
console.log(result);
c.close();
