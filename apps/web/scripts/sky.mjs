// Renders the Home hero's "Daylight" sky: a soft morning sky with clouds,
// slightly mint at the horizon (docs/WEBSITE.md, Home as built). No stock
// photography: the clouds are seeded fBm value noise with a little domain
// warping, lit from above, over a vertical sky gradient. Deterministic: the
// same seed always gives the same pixels, so the outputs can be committed.
//
//   node apps/web/scripts/sky.mjs
//
// Writes src/assets/sky.avif and src/assets/sky.webp (2800 x 1500, the hero
// at 2x for about 1400 CSS pixels). sharp comes with Astro; it is resolved
// through Astro's own dependency tree so the site needs no extra dependency.
import { createRequire } from "node:module";
import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const sharp = createRequire(require.resolve("astro/package.json"))("sharp");

const W = 2800;
const H = 1500;
const SEED = 20261108;

// --- Noise ------------------------------------------------------------------

function hash2(x, y) {
  let h = Math.imul(x, 374761393) ^ Math.imul(y, 668265263) ^ SEED;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
}

const smooth = (t) => t * t * t * (t * (t * 6 - 15) + 10);

function noise(x, y) {
  const xi = Math.floor(x);
  const yi = Math.floor(y);
  const u = smooth(x - xi);
  const v = smooth(y - yi);
  const a = hash2(xi, yi);
  const b = hash2(xi + 1, yi);
  const c = hash2(xi, yi + 1);
  const d = hash2(xi + 1, yi + 1);
  return a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v;
}

function fbm(x, y, octaves) {
  let sum = 0;
  let amp = 0.5;
  let norm = 0;
  for (let o = 0; o < octaves; o++) {
    sum += amp * noise(x, y);
    norm += amp;
    // Rotate a little each octave so the lattice never lines up.
    const nx = x * 1.6 - y * 1.2 + 17.3;
    y = x * 1.2 + y * 1.6 + 9.1;
    x = nx;
    amp *= 0.5;
  }
  return sum / norm;
}

const clamp = (v, a = 0, b = 1) => Math.max(a, Math.min(b, v));
const sstep = (a, b, v) => {
  const t = clamp((v - a) / (b - a));
  return t * t * (3 - 2 * t);
};
const mix = (a, b, t) => a + (b - a) * t;

// --- Sky ----------------------------------------------------------------------

// Top to bottom: soft blue, pale blue, a mint-tinted horizon, then the hero
// panel's fill so the image ends without an edge.
const STOPS = [
  [0.0, [168, 201, 230]],
  [0.38, [204, 225, 241]],
  [0.68, [226, 240, 240]],
  [0.86, [228, 243, 236]],
  [1.0, [238, 244, 241]],
];

function sky(v) {
  for (let i = 1; i < STOPS.length; i++) {
    const [p1, c1] = STOPS[i];
    const [p0, c0] = STOPS[i - 1];
    if (v <= p1) {
      const t = (v - p0) / (p1 - p0);
      return [mix(c0[0], c1[0], t), mix(c0[1], c1[1], t), mix(c0[2], c1[2], t)];
    }
  }
  return STOPS[STOPS.length - 1][1];
}

// Where clouds may form: a bank low in the frame, a few drifting high in the
// corners, thinner behind the headline in the middle.
function coverage(u, v) {
  const bank = sstep(0.42, 0.72, v) * (1 - sstep(0.9, 1.0, v));
  const high = (1 - sstep(0.05, 0.36, v)) * sstep(0.12, 0.42, Math.abs(u - 0.5));
  const centre = 1 - 0.55 * (1 - sstep(0.12, 0.34, Math.abs(u - 0.5))) * (1 - sstep(0.4, 0.62, v));
  return clamp(Math.max(bank * 0.95, high * 0.75, 0.22) * centre);
}

// Cloud density at a point (aspect-corrected units: 1 = the image height).
function density(x, y) {
  const s = 3.1;
  const wx = fbm(x * s * 0.7 + 3.1, y * s * 1.4 + 7.7, 3) - 0.5;
  const wy = fbm(x * s * 0.7 + 11.4, y * s * 1.4 + 1.9, 3) - 0.5;
  // Wider than tall, like cumulus seen from the ground.
  const n = fbm((x + wx * 0.35) * s, (y + wy * 0.22) * s * 1.9, 6);
  return n;
}

const data = Buffer.alloc(W * H * 3);
const aspect = W / H;
for (let py = 0; py < H; py++) {
  const v = py / (H - 1);
  const base = sky(v);
  for (let px = 0; px < W; px++) {
    const u = px / (W - 1);
    const x = u * aspect;
    const y = v;
    const cov = coverage(u, v);
    const d = density(x, y);
    const thresh = mix(0.63, 0.43, cov);
    const c = sstep(thresh, thresh + 0.13, d);
    let r = base[0];
    let g = base[1];
    let b = base[2];
    if (c > 0.001) {
      // Lit from above: thicker cloud just above this point means shade here.
      const above = density(x, y - 0.018);
      const shade = clamp((above - d) * 5.5 + 0.2 * (1 - c));
      const lit = 1 - 0.7 * shade;
      const cr = mix(203, 255, lit);
      const cg = mix(217, 255, lit);
      const cb = mix(229, 255, lit);
      const a = c * 0.94;
      r = mix(r, cr, a);
      g = mix(g, cg, a);
      b = mix(b, cb, a);
    }
    // A faint warm-white haze just above the horizon.
    const haze = Math.exp(-(((v - 0.8) / 0.12) ** 2)) * 0.18;
    r = mix(r, 246, haze);
    g = mix(g, 251, haze);
    b = mix(b, 248, haze);
    // Fade into the panel fill over the last tenth.
    const end = sstep(0.9, 1.0, v);
    r = mix(r, 238, end);
    g = mix(g, 244, end);
    b = mix(b, 241, end);
    const i = (py * W + px) * 3;
    data[i] = r;
    data[i + 1] = g;
    data[i + 2] = b;
  }
}

const out = join(here, "..", "src", "assets");
mkdirSync(out, { recursive: true });
const img = () => sharp(data, { raw: { width: W, height: H, channels: 3 } });
const avif = await img().avif({ quality: 62, effort: 9, chromaSubsampling: "4:2:0" }).toFile(join(out, "sky.avif"));
const webp = await img().webp({ quality: 80, effort: 6, smartSubsample: true }).toFile(join(out, "sky.webp"));
console.log(`sky: avif ${(avif.size / 1024).toFixed(1)} KB, webp ${(webp.size / 1024).toFixed(1)} KB`);
