// Copies the generated brand files that must be served from the site root
// (favicons, web manifest, Open Graph image) from brand/ into public/.
// brand/ is the single source; the copies in public/ are ignored by git.
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const brand = join(here, "..", "..", "..", "brand");
const pub = join(here, "..", "public");

const files = [
  ["web/favicon.ico", "favicon.ico"],
  ["web/favicon.svg", "favicon.svg"],
  ["web/apple-touch-icon.png", "apple-touch-icon.png"],
  ["web/icon-192.png", "icon-192.png"],
  ["web/icon-512.png", "icon-512.png"],
  ["web/icon-maskable-512.png", "icon-maskable-512.png"],
  ["web/site.webmanifest", "site.webmanifest"],
  ["social/og-image.png", "og-image.png"],
];

mkdirSync(pub, { recursive: true });
for (const [from, to] of files) {
  copyFileSync(join(brand, from), join(pub, to));
}
console.log(`copy-brand: ${files.length} files from brand/ to public/`);
