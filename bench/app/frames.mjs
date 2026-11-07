// Runs smooth.js in the app's page four times (progress only and flinging,
// at normal speed and with the CPU slowed 4x) and prints one JSON line each.
//   node frames.mjs
import fs from "node:fs";
import { connect } from "./cdp.mjs";

const src = fs.readFileSync(new URL("./smooth.js", import.meta.url), "utf8");
const c = await connect();
console.log("viewport", await c.evaluate("JSON.stringify([innerWidth, innerHeight, devicePixelRatio])"));
for (const [rate, fling] of [[1, false], [1, true], [4, false], [4, true]]) {
  await c.send("Emulation.setCPUThrottlingRate", { rate });
  await new Promise((r) => setTimeout(r, 500));
  const r = await c.evaluate(src.replace("__FLING__", String(fling)));
  console.log(JSON.stringify({ cpu: `${rate}x`, fling, ...r }));
}
await c.send("Emulation.setCPUThrottlingRate", { rate: 1 });
c.close();
