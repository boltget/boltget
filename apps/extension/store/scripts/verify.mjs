// Checks every PNG in output/ and captures/: the sizes the Chrome Web Store
// asks for, no alpha where it forbids it, and no metadata. A PNG may hold
// only the chunks needed to draw it (IHDR, PLTE, tRNS, IDAT, IEND, and sRGB
// or gAMA); anything else (tEXt, iTXt, zTXt, eXIf, iCCP, caBX and other
// C2PA/JUMBF provenance, time stamps) fails, and so does any trace of
// "c2pa", "jumb" or "xmp" in the bytes. Run on its own:
// `node store/scripts/verify.mjs`.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { CAPTURES, KEPT_CHUNKS, OUTPUT } from "./lib/paths.mjs";

const SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const TRACES = ["c2pa", "jumb", "jumd", "caBX", "xmp", "exif", "AI generated", "Software"];

/** What the store wants of each output: its size and whether alpha is allowed. */
const SPECS = [
  { match: /^screenshot-\d-.*\.png$/, width: 1280, height: 800, alpha: false },
  { match: /^promo-small-440x280\.png$/, width: 440, height: 280, alpha: false },
  { match: /^marquee-1400x560\.png$/, width: 1400, height: 560, alpha: false },
  { match: /^store-icon-128\.png$/, width: 128, height: 128, alpha: true },
];
const EXPECTED_OUTPUTS = 8;

function chunks(bytes) {
  if (!bytes.subarray(0, 8).equals(SIGNATURE)) throw new Error("not a PNG");
  const out = [];
  for (let at = 8; at < bytes.length;) {
    const length = bytes.readUInt32BE(at);
    const type = bytes.toString("latin1", at + 4, at + 8);
    out.push({ type, data: bytes.subarray(at + 8, at + 8 + length) });
    at += 12 + length;
    if (type === "IEND") {
      if (at !== bytes.length) throw new Error("bytes after IEND");
      break;
    }
  }
  return out;
}

function check(path, spec) {
  const problems = [];
  const bytes = readFileSync(path);
  const list = chunks(bytes);
  for (const { type } of list) if (!KEPT_CHUNKS.has(type)) problems.push(`chunk ${type}`);
  // Only the non-pixel bytes can carry text; compressed IDAT may match by chance.
  const meta = Buffer.concat(list.filter((c) => c.type !== "IDAT").map((c) => c.data));
  for (const trace of TRACES) {
    if (meta.toString("latin1").toLowerCase().includes(trace.toLowerCase())) {
      problems.push(`"${trace}" in metadata`);
    }
  }
  const ihdr = list[0];
  if (ihdr?.type !== "IHDR") problems.push("IHDR is not first");
  else if (spec) {
    const width = ihdr.data.readUInt32BE(0);
    const height = ihdr.data.readUInt32BE(4);
    const colorType = ihdr.data[9];
    if (width !== spec.width || height !== spec.height) {
      problems.push(`${width}x${height}, wants ${spec.width}x${spec.height}`);
    }
    const alpha = colorType === 4 || colorType === 6 || list.some((c) => c.type === "tRNS");
    if (alpha && !spec.alpha) problems.push("has alpha");
  }
  return problems;
}

/** Verifies the images; throws listing every problem. */
export function verify() {
  const failures = [];
  const outputs = readdirSync(OUTPUT).filter((f) => f.endsWith(".png"));
  if (outputs.length !== EXPECTED_OUTPUTS) {
    failures.push(`output/ holds ${outputs.length} PNGs, wants ${EXPECTED_OUTPUTS}`);
  }
  for (const file of outputs) {
    const spec = SPECS.find((s) => s.match.test(file));
    if (!spec) failures.push(`output/${file}: not a store image`);
    for (const p of check(join(OUTPUT, file), spec)) failures.push(`output/${file}: ${p}`);
  }
  for (const file of readdirSync(CAPTURES).filter((f) => f.endsWith(".png"))) {
    for (const p of check(join(CAPTURES, file))) failures.push(`captures/${file}: ${p}`);
  }
  const others = [...readdirSync(OUTPUT), ...readdirSync(CAPTURES)].filter(
    (f) => !f.endsWith(".png"),
  );
  for (const f of others) failures.push(`${f}: not a PNG`);
  if (failures.length) throw new Error(`store images failed:\n  ${failures.join("\n  ")}`);
  console.log(`verified ${outputs.length} store images and the captures: no metadata`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  try {
    verify();
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
