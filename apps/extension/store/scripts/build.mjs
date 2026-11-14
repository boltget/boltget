// Builds every Chrome Web Store image (store/listing.md says where each goes):
//
//   1. captures the real extension popup and options page (release build,
//      against the end-to-end stub host) and the real desktop app UI (its dev
//      server with the simulated engine) into captures/;
//   2. composes the five screenshots, the small promo tile and the marquee on
//      the pearl sky, and the 128 px store icon, into output/;
//   3. verifies sizes, alpha and that no metadata is left.
//
// `pnpm --filter @boltget/extension store-images` runs all of it;
// `... store-images --compose` skips the captures and recomposes from captures/.
import { rmSync } from "node:fs";
import { captureApp } from "./capture-app.mjs";
import { captureExtension } from "./capture-extension.mjs";
import { compose, storeIcon } from "./compose.mjs";
import { OUTPUT } from "./lib/paths.mjs";
import { verify } from "./verify.mjs";

const composeOnly = process.argv.includes("--compose");

if (!composeOnly) {
  console.log("Capturing the extension (release build)…");
  await captureExtension();
  console.log("Capturing the desktop app (simulated engine)…");
  await captureApp();
}
console.log("Composing the store images…");
rmSync(OUTPUT, { recursive: true, force: true });
for (const file of await compose()) console.log(`  ${file}`);
console.log(`  ${await storeIcon()}`);
verify();
