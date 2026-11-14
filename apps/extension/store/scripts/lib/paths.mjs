// Shared paths and helpers for the store image scripts.
import { execSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";

const at = (path) => fileURLToPath(new URL(path, import.meta.url));

export const STORE_DIR = at("../../");
export const EXTENSION_DIR = at("../../../");
export const REPO_DIR = at("../../../../../");
/** Raw captures of the real UI (kept, so slides can be recomposed without a browser run). */
export const CAPTURES = at("../../captures/");
/** The images to upload. */
export const OUTPUT = at("../../output/");
/** Scratch space: the patched build, the browser profile, rendered pages. Ignored by git. */
export const CACHE = at("../../.cache/");
/** Device pixels per CSS pixel for every capture and render. */
export const SCALE = 2;

/** Runs a fixed command line (no user input) in `cwd`, inheriting the output; throws when it fails. */
export function run(command, args, cwd) {
  execSync([command, ...args].join(" "), { cwd, stdio: "inherit" });
}

/** The PNG chunks a store image keeps: what it takes to draw it, nothing else. */
export const KEPT_CHUNKS = new Set(["IHDR", "PLTE", "tRNS", "IDAT", "IEND", "sRGB", "gAMA"]);

/** Drops every PNG chunk not in KEPT_CHUNKS (each chunk carries its own CRC). */
export function stripChunks(png) {
  const parts = [png.subarray(0, 8)];
  for (let at = 8; at < png.length;) {
    const length = png.readUInt32BE(at);
    const type = png.toString("latin1", at + 4, at + 8);
    if (KEPT_CHUNKS.has(type)) parts.push(png.subarray(at, at + 12 + length));
    at += 12 + length;
    if (type === "IEND") break;
  }
  return Buffer.concat(parts);
}

/**
 * Writes a PNG without metadata: re-encoded by sharp without `withMetadata`
 * (no EXIF, XMP, ICC or text), then cut to KEPT_CHUNKS (sharp still writes
 * pHYs). `opaque` flattens onto white and drops the alpha channel, as the
 * stores want for screenshots and tiles.
 */
export async function savePng(input, path, { opaque = false, resize } = {}) {
  mkdirSync(dirname(path), { recursive: true });
  let image = sharp(input).toColourspace("srgb");
  if (resize) image = image.resize({ ...resize, kernel: "lanczos3" });
  if (opaque) image = image.flatten({ background: "#ffffff" }).removeAlpha();
  const png = await image.png({ compressionLevel: 9, adaptiveFiltering: true }).toBuffer();
  writeFileSync(path, stripChunks(png));
}
