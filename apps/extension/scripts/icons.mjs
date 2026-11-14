// Copies the toolbar and store icons from the brand kit (brand/png, generated
// by brand/scripts/build.mjs) into public/icons: Onyx for light toolbars and
// the stores, its Paper twin for dark toolbars. `--check` fails when the copies
// are stale instead of writing them.
import { mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";

const SIZES = [16, 32, 48, 128];
const VARIANTS = ["icon", "icon-on-dark"];
const brand = new URL("../../../brand/png/", import.meta.url);
const out = new URL("../public/icons/", import.meta.url);
const check = process.argv.includes("--check");

mkdirSync(out, { recursive: true });
let stale = [];
for (const variant of VARIANTS) {
  for (const size of SIZES) {
    const name = `${variant}-${size}.png`;
    const source = readFileSync(new URL(name, brand));
    const target = new URL(name, out);
    const same = existsSync(target) && readFileSync(target).equals(source);
    if (same) continue;
    if (check) stale.push(name);
    else writeFileSync(target, source);
  }
}
if (stale.length > 0) {
  console.error(
    `public/icons is stale (${stale.join(", ")}): run pnpm --filter @boltget/extension icons`,
  );
  process.exit(1);
}
