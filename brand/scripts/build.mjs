#!/usr/bin/env node
/**
 * Boltget brand kit build.
 *
 * Regenerates every asset in this folder (icon.svg, icon-animated.svg, SVG,
 * PNG, ICO, social images, tokens, preview.html) from the geometry and
 * palette defined below. Edit this script, never the outputs.
 *
 *   pnpm install && pnpm build
 */
import { mkdirSync, readFileSync, writeFileSync, copyFileSync, rmSync } from 'node:fs';
import { dirname, join, resolve, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Resvg } from '@resvg/resvg-js';
import opentype from 'opentype.js';
import { create as openFont } from 'fontkitten';
import sharp from 'sharp';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const REPO = resolve(ROOT, '..');
const FONT_DIR = join(ROOT, 'node_modules', 'geist', 'dist', 'fonts');
/** Vendored font inputs that no npm package here provides (source/fonts). */
const SOURCE_FONTS = join(ROOT, 'source', 'fonts');
const out = (...p) => join(ROOT, ...p);

/* ------------------------------------------------------------------------ */
/* 1. Brand constants                                                        */
/* ------------------------------------------------------------------------ */

/** The mark: a down-arrow on a 9x9 pixel grid. [firstCol, lastCol] per row. */
const ROWS = [
  [3, 5], [3, 5], [3, 5], [3, 5], // shaft
  [0, 8], [1, 7], [2, 6], [3, 5], [4, 4], // head
];
const CELLS = ROWS.flatMap(([a, b], r) => Array.from({ length: b - a + 1 }, (_, i) => ({ r, c: a + i })));

/**
 * The app icon's master geometry, on a 1024 artboard: a smooth-corner
 * (squircle) tile and the arrow's 37 pixels, 50 wide on a 58 pitch (8 gaps),
 * each with a 6 corner.
 */
const MASTER = { size: 1024, inset: 40, radius: 208, smoothing: 0.6, pitch: 58, px: 50, corner: 6 };

/**
 * The mark without a tile (lockups, the website, the app's empty state) keeps
 * square pixels on a 180 artboard: grid origin 36, pitch u = 12, pixel 10.
 */
const ART = 180;
const GRID = { origin: 36, pitch: 12, inset: 1, px: 10, n: 9 };
const U = GRID.pitch;

/** Animation signature (BRAND.md, Motion). */
const MOTION = {
  dur: 2.6, // s, full loop
  stagger: 0.12, // s, per row
  values: [0.15, 1, 1, 0.15],
  keyTimes: [0, 0.12, 0.85, 1],
};

/**
 * Type (BRAND.md, Typography). Red Hat Display for headings and large
 * figures, Red Hat Text for everything else, Geist Mono for code. The text
 * face reads thin at its regular weight, so body copy sits at 450. The
 * wordmark is outlined Geist SemiBold and does not follow these.
 */
const FONT = {
  display: { family: 'Red Hat Display', weight: 600, tracking: -0.02 },
  text: { family: 'Red Hat Text', weight: 450 },
  mono: { family: 'Geist Mono', weight: 400 },
};

/*
 * Monochrome. Black and white carry the brand; graphite carries structure.
 * Colour appears only to say a download is paused or has failed.
 */
/** Ink: the Onyx tile, the one-colour mark, text and the app's accent on light. */
const INK = '#0A0A0B';
/** Paper: the warm off-white of the website and printed matter. */
const PAPER = '#F5F5F3';
const WHITE = '#FFFFFF';
const GRAPHITE = {
  50: '#F7F7F8', 100: '#EEEFF0', 200: '#DEDFE1', 300: '#C4C6C9', 400: '#A0A3A8', 500: '#7D8086',
  600: '#5F6267', 700: '#46484C', 800: '#2E3033', 900: '#1C1D1F', 950: '#111213',
};
/** State colours, the only colour in the system: paused (amber) and failed (red). */
const SEMANTIC = {
  paused: { light: '#B45309', dark: '#F5A524' },
  error: { light: '#DC2626', dark: '#F87171' },
};
const THEME = {
  dark: {
    bg: INK, surface: GRAPHITE[950], raised: GRAPHITE[900], border: GRAPHITE[800],
    'border-strong': GRAPHITE[700], text: GRAPHITE[50], 'text-muted': GRAPHITE[400],
    'text-subtle': GRAPHITE[500], accent: WHITE, 'accent-hover': GRAPHITE[200],
    'on-accent': INK, 'accent-subtle': '#FFFFFF1A',
  },
  light: {
    bg: WHITE, surface: GRAPHITE[50], raised: GRAPHITE[100], border: GRAPHITE[200],
    'border-strong': GRAPHITE[300], text: INK, 'text-muted': GRAPHITE[700],
    'text-subtle': GRAPHITE[600], accent: INK, 'accent-hover': GRAPHITE[800],
    'on-accent': WHITE, 'accent-subtle': '#0A0A0B14',
  },
};

/**
 * The app icon has two modes (BRAND.md, The app icon). Onyx, the primary, is
 * a black tile lit from the top left with a white arrow, for light contexts.
 * Paper, its inverted twin, is a white tile with a black arrow, for dark
 * contexts (a dark taskbar, dark tabs, the app's dark theme).
 * tile: the 120 degree tile gradient; arrow: each pixel's top-to-foot ramp;
 * small: the flat tile, its 1 px edge and the arrow below 48 px.
 */
const ICON = {
  onyx: {
    tile: ['#2C2D31', '#151618', INK],
    arrow: [[0, WHITE], [1, '#E4E5E8']],
    small: { tile: ['#26272A', '#0E0E10'], edge: [WHITE, 0.14], arrow: WHITE, lit: 0.12 },
  },
  paper: {
    tile: [WHITE, '#F3F4F5', '#E1E3E6'],
    arrow: [[0, '#26282B'], [0.35, '#121315'], [1, '#08090A']],
    small: { tile: [WHITE, '#E4E6E9'], edge: ['#000000', 0.22], arrow: INK, lit: 0 },
  },
};

/**
 * Atmosphere (website only, BRAND.md "Atmosphere (website)"): the pearl sky,
 * a grey cloud photograph washed toward paper, top to bottom.
 */
const ATMOSPHERE = {
  haze: GRAPHITE[300], pearl: GRAPHITE[200], mist: GRAPHITE[100], paper: PAPER, white: WHITE,
};

/** Progress fill (the app's running downloads): graphite rising to the accent at the head. */
const PROGRESS_DARK = [[0, GRAPHITE[500]], [0.5, GRAPHITE[300]], [1, WHITE]];
const PROGRESS_LIGHT = [[0, GRAPHITE[500]], [0.5, GRAPHITE[700]], [1, INK]];

/** Copy. */
const COPY = {
  headline: ['Every byte,', 'at full speed.'],
  subline: 'The fast, resumable download manager.',
};

/* ------------------------------------------------------------------------ */
/* 2. Colour and geometry helpers                                            */
/* ------------------------------------------------------------------------ */

const fmt = (n) => +n.toFixed(3);
const toHex = (rgb) => '#' + rgb.map((v) => Math.round(Math.max(0, Math.min(255, v))).toString(16).padStart(2, '0')).join('').toUpperCase();

/** Figma-style smooth-corner rectangle (corner smoothing s, 0..1). */
function squircle(x, y, w, h, r, s = MASTER.smoothing) {
  const p = (1 + s) * r;
  const rad = (deg) => (deg * Math.PI) / 180;
  const arcMeasure = 90 * (1 - s);
  const arc = Math.sin(rad(arcMeasure / 2)) * r * Math.SQRT2;
  const alpha = (90 - arcMeasure) / 2;
  const p34 = r * Math.tan(rad(alpha / 2));
  const beta = 45 * s;
  const c = p34 * Math.cos(rad(beta));
  const d = c * Math.tan(rad(beta));
  const b = (p - arc - c - d) / 3;
  const a = 2 * b;
  const f = (n) => +n.toFixed(2);
  return [
    `M${f(x + w - p)} ${f(y)}`,
    `c${f(a)} 0 ${f(a + b)} 0 ${f(a + b + c)} ${f(d)}a${f(r)} ${f(r)} 0 0 1 ${f(arc)} ${f(arc)}c${f(d)} ${f(c)} ${f(d)} ${f(b + c)} ${f(d)} ${f(a + b + c)}`,
    `L${f(x + w)} ${f(y + h - p)}`,
    `c0 ${f(a)} 0 ${f(a + b)} ${f(-d)} ${f(a + b + c)}a${f(r)} ${f(r)} 0 0 1 ${f(-arc)} ${f(arc)}c${f(-c)} ${f(d)} ${f(-(b + c))} ${f(d)} ${f(-(a + b + c))} ${f(d)}`,
    `L${f(x + p)} ${f(y + h)}`,
    `c${f(-a)} 0 ${f(-(a + b))} 0 ${f(-(a + b + c))} ${f(-d)}a${f(r)} ${f(r)} 0 0 1 ${f(-arc)} ${f(-arc)}c${f(-d)} ${f(-c)} ${f(-d)} ${f(-(b + c))} ${f(-d)} ${f(-(a + b + c))}`,
    `L${f(x)} ${f(y + p)}`,
    `c0 ${f(-a)} 0 ${f(-(a + b))} ${f(d)} ${f(-(a + b + c))}a${f(r)} ${f(r)} 0 0 1 ${f(arc)} ${f(-arc)}c${f(c)} ${f(-d)} ${f(b + c)} ${f(-d)} ${f(a + b + c)} ${f(-d)}`,
    'Z',
  ].join('');
}

/** Path of the square-pixel mark (one subpath per pixel) on the 180 grid. */
function markPath(g = GRID, cells = CELLS) {
  return cells
    .map(({ r, c }) => `M${fmt(g.origin + c * g.pitch + g.inset)} ${fmt(g.origin + r * g.pitch + g.inset)}h${fmt(g.px)}v${fmt(g.px)}h${fmt(-g.px)}z`)
    .join('');
}

/**
 * Resolution-reduced arrows for small sizes: the same arrow on a 5x5, 7x7 or
 * 9x9 grid, so every cell and every gap lands on whole device pixels.
 * [firstCol, lastCol] per row.
 */
const GRIDS = {
  5: [[2, 2], [2, 2], [0, 4], [1, 3], [2, 2]],
  7: [[2, 4], [2, 4], [2, 4], [0, 6], [1, 5], [2, 4], [3, 3]],
  9: ROWS,
};

/**
 * An n-grid arrow in device pixels with integer cells and 1 px (or wider)
 * gaps, centred in a `size` square. When the arrow's width is odd the centre
 * column is 1 px wider, so the arrow centres on even sizes.
 */
function pixelCells(size, [n, cell, gap]) {
  const mid = (n - 1) / 2;
  const odd = (n * cell + (n - 1) * gap) % 2 === 1;
  const cw = (c) => cell + (odd && c === mid ? 1 : 0);
  const xs = [];
  let x = 0;
  for (let c = 0; c < n; c++) { xs.push(x); x += cw(c) + gap; }
  const w = x - gap, h = n * cell + (n - 1) * gap;
  const ox = Math.round((size - w) / 2), oy = Math.floor((size - h) / 2);
  return GRIDS[n].flatMap(([a, b], r) =>
    Array.from({ length: b - a + 1 }, (_, i) => ({ r, x: ox + xs[a + i], y: oy + r * (cell + gap), w: cw(a + i), h: cell })));
}
const pixelRects = (cells, fill = '') =>
  cells.map(({ x, y, w, h }) => `<rect x="${x}" y="${y}" width="${w}" height="${h}"${fill && ` fill="${fill}"`}/>`).join('');

/* ------------------------------------------------------------------------ */
/* 3. SVG helpers                                                            */
/* ------------------------------------------------------------------------ */

function svg({ w, h, viewBox = `0 0 ${w} ${h}`, title, defs = '', body, style = '' }) {
  return [
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${viewBox}" width="${w}" height="${h}">`,
    `  <title>${title}</title>`,
    style && `  <style>${style}</style>`,
    defs && `  <defs>\n${defs}\n  </defs>`,
    body,
    '</svg>',
    '',
  ]
    .filter(Boolean)
    .join('\n');
}
const stops = (list) => list.map(([o, c, a]) => `<stop offset="${o}" stop-color="${c}"${a === undefined ? '' : ` stop-opacity="${a}"`}/>`).join('');
/** Vertical gradient in user space, y0 -> y1. */
const vGrad = (id, list, y0, y1) => `    <linearGradient id="${id}" gradientUnits="userSpaceOnUse" x1="0" y1="${fmt(y0)}" x2="0" y2="${fmt(y1)}">${stops(list)}</linearGradient>`;
/** The tile's 120 degree CSS-style gradient across a square box. */
const tileGrad = (id, list) => `    <linearGradient id="${id}" x1="0.067" y1="0.25" x2="0.933" y2="0.75">${stops(list.map((c, i) => [i / (list.length - 1), c]))}</linearGradient>`;

/* ------------------------------------------------------------------------ */
/* 4. Type: Geist (opentype.js) and Source Serif 4 (fontkitten), outlined    */
/* ------------------------------------------------------------------------ */

const fonts = {};
const font = (file) => (fonts[file] ??= opentype.loadSync(join(FONT_DIR, file)));
const SANS = (w) => font(`geist-sans/Geist-${w}.ttf`);
const MONO = (w) => font(`geist-mono/GeistMono-${w}.ttf`);
/** Source Serif 4: the upright is a brand input only, the italic also ships on the site. fontkitten reads woff2. */
const SERIF = (italic = false) =>
  (fonts[`serif-${italic}`] ??= openFont(readFileSync(italic ? join(REPO, 'apps', 'web', 'src', 'fonts', 'SourceSerif4-400-italic-latin.woff2') : join(SOURCE_FONTS, 'SourceSerif4-400-latin.woff2'))));

/** Optical kerning for the wordmark, in font units, added to the font's own GPOS kerning. */
const WORDMARK = { weight: 'SemiBold', tracking: -0.025, adjust: { Bo: -6, lt: 12 } };

/**
 * Lay out a single line as outlined path data.
 * anchor: 'start' | 'middle' | 'end'. Returns { d, width, bbox }.
 */
function text(str, { f, size, x = 0, y = 0, tracking = 0, adjust = {}, anchor = 'start', decimals = 2 }) {
  if (!f.stringToGlyphs) return serifText(str, { f, size, x, y, tracking, anchor, decimals });
  const glyphs = f.stringToGlyphs(str);
  const s = size / f.unitsPerEm;
  const chars = [...str];
  const advances = [];
  let width = 0;
  glyphs.forEach((g, i) => {
    let adv = g.advanceWidth * s;
    if (i < glyphs.length - 1) {
      adv += tracking * size;
      adv += (f.getKerningValue(g, glyphs[i + 1]) + (adjust[chars[i] + chars[i + 1]] ?? 0)) * s;
    }
    advances.push(adv);
    width += adv;
  });
  let cx = anchor === 'middle' ? x - width / 2 : anchor === 'end' ? x - width : x;
  const startX = cx;
  let d = '';
  let x1 = Infinity, y1 = Infinity, x2 = -Infinity, y2 = -Infinity;
  glyphs.forEach((g, i) => {
    const p = g.getPath(cx, y, size);
    d += p.toPathData(decimals);
    const bb = p.getBoundingBox();
    if (bb.x1 < bb.x2) {
      x1 = Math.min(x1, bb.x1); y1 = Math.min(y1, bb.y1);
      x2 = Math.max(x2, bb.x2); y2 = Math.max(y2, bb.y2);
    }
    cx += advances[i];
  });
  return { d, width, x: startX, bbox: { x1, y1, x2, y2 } };
}
function serifText(str, { f, size, x, y, tracking, anchor, decimals }) {
  const glyphs = f.glyphsForString(str);
  const s = size / f.unitsPerEm;
  const adv = glyphs.map((g, i) => g.advanceWidth * s + (i < glyphs.length - 1 ? tracking * size : 0));
  const width = adv.reduce((a, b) => a + b, 0);
  let cx = anchor === 'middle' ? x - width / 2 : anchor === 'end' ? x - width : x;
  const startX = cx;
  const n = (v) => +v.toFixed(decimals);
  let d = '';
  let x2 = -Infinity;
  glyphs.forEach((g, i) => {
    const X = (v) => n(cx + v * s), Y = (v) => n(y - v * s);
    for (const { command, args } of g.path.commands) {
      if (command === 'moveTo') d += `M${X(args[0])} ${Y(args[1])}`;
      else if (command === 'lineTo') d += `L${X(args[0])} ${Y(args[1])}`;
      else if (command === 'quadraticCurveTo') d += `Q${X(args[0])} ${Y(args[1])} ${X(args[2])} ${Y(args[3])}`;
      else if (command === 'bezierCurveTo') d += `C${X(args[0])} ${Y(args[1])} ${X(args[2])} ${Y(args[3])} ${X(args[4])} ${Y(args[5])}`;
      else if (command === 'closePath') d += 'Z';
    }
    const bb = g.bbox;
    if (bb && bb.maxX > bb.minX) x2 = Math.max(x2, cx + bb.maxX * s);
    cx += adv[i];
  });
  return { d, width, x: startX, bbox: { x1: startX, y1: y - size, x2, y2: y } };
}
const capHeight = (f, size) => (f.tables?.os2 ? f.tables.os2.sCapHeight : f.capHeight) / f.unitsPerEm * size;

function wordmarkText(size, x = 0, y = 0, anchor = 'start') {
  return text('Boltget', { f: SANS(WORDMARK.weight), size, x, y, tracking: WORDMARK.tracking, adjust: WORDMARK.adjust, anchor });
}

/* ------------------------------------------------------------------------ */
/* 5. Raster + ICO helpers                                                   */
/* ------------------------------------------------------------------------ */

function render(svgText, width, height) {
  const r = new Resvg(svgText, {
    fitTo: { mode: 'width', value: width },
    font: { loadSystemFonts: false },
    shapeRendering: 2,
  }).render();
  if (height && r.height !== height) throw new Error(`render size mismatch ${r.width}x${r.height}`);
  return r;
}
const dataUri = (buf) => 'data:image/png;base64,' + Buffer.from(buf).toString('base64');

/** ICO with 32-bit BMP entries below 256 px (max compatibility) and PNG for 256. */
function encodeIco(images /* [{size, rendered}] */) {
  const blobs = images.map(({ size, rendered }) => {
    if (size >= 256) return rendered.asPng();
    const px = rendered.pixels; // premultiplied RGBA, top-down
    const maskStride = Math.ceil(size / 32) * 4;
    const buf = Buffer.alloc(40 + size * size * 4 + maskStride * size);
    buf.writeUInt32LE(40, 0);
    buf.writeInt32LE(size, 4);
    buf.writeInt32LE(size * 2, 8);
    buf.writeUInt16LE(1, 12);
    buf.writeUInt16LE(32, 14);
    buf.writeUInt32LE(size * size * 4 + maskStride * size, 20);
    let o = 40;
    for (let y = size - 1; y >= 0; y--) {
      for (let x = 0; x < size; x++) {
        const i = (y * size + x) * 4;
        const a = px[i + 3];
        const un = (v) => (a === 0 ? 0 : Math.min(255, Math.round((v * 255) / a)));
        buf[o++] = un(px[i + 2]);
        buf[o++] = un(px[i + 1]);
        buf[o++] = un(px[i]);
        buf[o++] = a;
      }
    }
    // AND mask left at 0 (alpha channel is authoritative)
    return buf;
  });
  const header = Buffer.alloc(6 + 16 * images.length);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(images.length, 4);
  let offset = header.length;
  images.forEach(({ size }, i) => {
    const e = 6 + i * 16;
    header[e] = size >= 256 ? 0 : size;
    header[e + 1] = size >= 256 ? 0 : size;
    header.writeUInt16LE(1, e + 4);
    header.writeUInt16LE(32, e + 6);
    header.writeUInt32LE(blobs[i].length, e + 8);
    header.writeUInt32LE(offset, e + 12);
    offset += blobs[i].length;
  });
  return Buffer.concat([header, ...blobs]);
}

/* ------------------------------------------------------------------------ */
/* 6. Output bookkeeping                                                     */
/* ------------------------------------------------------------------------ */

const written = [];
function write(path, data) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, data);
  written.push(relative(ROOT, path).replaceAll('\\', '/'));
}
const svgs = {}; // name -> svg text (for rasterising)
function writeSvg(rel, txt) {
  svgs[rel] = txt;
  write(out(rel), txt);
}
function writePng(rel, svgText, w, h) {
  const r = render(svgText, w, h);
  write(out(rel), r.asPng());
  return r;
}

/* ------------------------------------------------------------------------ */
/* 7. The app icon                                                           */
/* ------------------------------------------------------------------------ */

/**
 * The master icon, drawn natively at `size` px. Onyx: a black tile lit from
 * the top left (120 degree gradient, a soft light at the top, a bright top
 * rim, a faint bottom rim and a 1 px outer edge) and a white arrow whose
 * pixels darken a touch toward the foot, on a soft contact shadow. Paper, its
 * twin for dark contexts: a white tile with a sheen at the top left and a
 * shaded foot, and a black arrow with a lit top edge. The pixel grid is
 * snapped to whole device pixels at every size.
 *
 * bleed: fill the whole square (apple-touch, avatars, maskable); the arrow
 * keeps its size relative to the visible tile. animated: the rows light up
 * top to bottom (SMIL, the brand signature).
 */
function masterSvg(size, { mode = 'onyx', bleed = false, animated = false, title = 'Boltget' } = {}) {
  const look = ICON[mode];
  const onyx = mode === 'onyx';
  const s = size / MASTER.size;
  const inset = bleed ? 0 : MASTER.inset * s;
  const TW = size - 2 * inset;
  const k = bleed ? MASTER.size / (MASTER.size - 2 * MASTER.inset) : 1;
  const tile = bleed ? `M0 0H${size}V${size}H0Z` : squircle(inset, inset, TW, TW, MASTER.radius * s);
  // Snap the grid: integer pitch and gap, centred.
  const exactPitch = MASTER.pitch * s * k;
  const pitch = size === MASTER.size && !bleed ? MASTER.pitch : Math.round(exactPitch);
  const gapRatio = (MASTER.pitch - MASTER.px) / MASTER.pitch;
  const gap = size === MASTER.size && !bleed ? MASTER.pitch - MASTER.px : Math.max(1, Math.round(pitch * gapRatio));
  const px = pitch - gap;
  const span = 8 * pitch + px;
  const O = Math.round((size - span) / 2);
  const corner = fmt(Math.max(0, (MASTER.corner / MASTER.px) * px));
  const at = (i) => O + i * pitch;
  const cell = ({ r, c }) => `<rect x="${at(c)}" y="${at(r)}" width="${px}" height="${px}" rx="${corner}"/>`;
  const sh = fmt(Math.max(0.6, (onyx ? 9 : 8) * s * k)); // shadow offset
  const edge = fmt(Math.max(0.5, 2.5 * s * k)); // lit top edge (Paper) or shaded foot (Onyx)
  const tileFill = onyx
    ? tileGrad('tile', look.tile)
    : vGrad('tile', look.tile.map((c, i) => [[0, 0.55, 1][i], c]), inset, inset + TW);

  const defs = [
    tileFill,
    `    <clipPath id="tc"><path d="${tile}"/></clipPath>`,
    `    <filter id="sh" x="0" y="0" width="${size}" height="${size}" filterUnits="userSpaceOnUse"><feGaussianBlur stdDeviation="${fmt(6 * s * k)}"/></filter>`,
    `    <linearGradient id="px" x1="0" y1="0" x2="0" y2="1">${stops(look.arrow)}</linearGradient>`,
  ];
  let light;
  if (onyx) {
    defs.push(
      `    <radialGradient id="lit" gradientUnits="userSpaceOnUse" cx="${fmt(inset + TW * 0.36)}" cy="${fmt(inset - TW * 0.13)}" r="${fmt(TW * 0.8)}">${stops([[0, WHITE, 0.16], [1, WHITE, 0]])}</radialGradient>`,
      vGrad('rimT', [[0, WHITE, 0.32], [1, WHITE, 0]], inset, inset + TW * 0.45),
      vGrad('rimB', [[0, WHITE, 0], [1, WHITE, 0.08]], inset + TW * 0.7, inset + TW),
    );
    light = `<rect width="${size}" height="${size}" fill="url(#lit)"/><path d="${tile}" fill="none" stroke="url(#rimT)" stroke-width="${fmt(5 * s)}"/><path d="${tile}" fill="none" stroke="url(#rimB)" stroke-width="${fmt(3 * s)}"/>`;
  } else {
    defs.push(
      `    <radialGradient id="lit" gradientUnits="userSpaceOnUse" cx="${fmt(inset + TW * 0.25)}" cy="${fmt(inset + TW * 0.085)}" r="${fmt(TW * 0.68)}">${stops([[0, WHITE, 1], [1, WHITE, 0]])}</radialGradient>`,
      vGrad('rimB', [[0, '#000', 0], [1, '#000', 0.1]], inset + TW * 0.6, inset + TW),
    );
    light = `<rect width="${size}" height="${size}" fill="url(#lit)" opacity="0.8"/><path d="${tile}" fill="none" stroke="url(#rimB)" stroke-width="${fmt(5 * s)}"/>`;
  }
  // Per row: its contact shadow, pixels and edge, so a row can fade as one in
  // the animated icon.
  const rowLayer = (r) => {
    const cells = CELLS.filter((c) => c.r === r).map(cell).join('');
    const dy = onyx ? -edge : edge;
    defs.push(`    <mask id="e${r}" maskUnits="userSpaceOnUse" x="0" y="0" width="${size}" height="${size}"><g fill="#fff">${cells}</g><g fill="#000" transform="translate(0 ${dy})">${cells}</g></mask>`);
    const parts = [
      `<g filter="url(#sh)" transform="translate(0 ${sh})" ${onyx ? 'fill="#000" fill-opacity="0.6"' : 'fill="#3A3D44" fill-opacity="0.3"'}>${cells}</g>`,
      `<g fill="url(#px)">${cells}</g>`,
      `<rect width="${size}" height="${size}" ${onyx ? 'fill="#9A9DA3" fill-opacity="0.35"' : 'fill="#fff" fill-opacity="0.14"'} mask="url(#e${r})"/>`,
    ].join('');
    if (!animated) return `  <g>${parts}</g>`;
    const begin = +(r * MOTION.stagger).toFixed(2);
    return `  <g opacity="${MOTION.values[0]}"><animate attributeName="opacity" values="${MOTION.values.join(';')}" keyTimes="${MOTION.keyTimes.join(';')}" dur="${MOTION.dur}s" begin="${begin}s" repeatCount="indefinite"/>${parts}</g>`;
  };
  const rows = ROWS.map((_, r) => rowLayer(r));
  // The 1 px outer edge: under the tile on Onyx, over it on Paper (where it
  // is what separates a white tile from a white page).
  const outline = `  <path d="${tile}" fill="none" stroke="#000" stroke-opacity="${onyx ? 0.3 : 0.14}" stroke-width="${fmt(2 * s)}"/>`;
  const body = [
    bleed || !onyx ? '' : outline,
    `  <path d="${tile}" fill="url(#tile)"/>`,
    `  <g clip-path="url(#tc)">${light}</g>`,
    bleed || onyx ? '' : outline,
    ...rows,
  ].filter(Boolean).join('\n');
  return svg({ w: size, h: size, title, defs: defs.join('\n'), body });
}

/**
 * Small sizes (16 to 128 px): [grid, cell, gap] per size, a resolution-reduced
 * arrow with real 1 px gaps (see GRIDS). The arrow fills more of the tile the
 * smaller the icon, so it stays legible on a taskbar or a tab.
 */
const SMALL_ARROW = {
  16: [5, 2, 1], 20: [5, 2, 1], 24: [7, 2, 1], 30: [9, 2, 1], 32: [9, 2, 1], 36: [9, 2, 1], 40: [9, 2, 1],
  48: [9, 3, 1], 60: [9, 4, 1], 64: [9, 4, 1], 72: [9, 4, 1], 80: [9, 5, 1], 96: [9, 5, 1], 128: [9, 7, 1],
};
/** The bare arrow (no tile) for the tray and the favicon: it fills the square. */
const BARE_ARROW = { 16: [5, 2, 1], 20: [7, 2, 1], 24: [7, 2, 1], 32: [9, 2, 1] };

/**
 * 16 to 40 px: a flat rounded tile with a 1 px edge (it keeps Onyx visible on
 * a dark taskbar and Paper on a light one) and, on Onyx, a 1 px lit top line.
 * 48 to 128 px: the squircle tile (inset 2 px, 5 from 96) with its gradient
 * and rim, and the same flat arrow. `arrow` overrides the size's recipe. crispEdges keeps the arrow on whole device pixels when a page places
 * the icon at a half-pixel offset.
 */
function smallSvg(size, mode = 'onyx', arrow = SMALL_ARROW[size]) {
  const look = ICON[mode];
  const onyx = mode === 'onyx';
  const t = look.small;
  const cells = pixelRects(pixelCells(size, arrow), t.arrow);
  const label = `Boltget (${size} px${onyx ? '' : ', for dark backgrounds'})`;
  if (size < 48) {
    const rx = size <= 16 ? 3 : size <= 24 ? 4 : Math.round(0.22 * size);
    const defs = vGrad('st', [[0, t.tile[0]], [1, t.tile[1]]], 0, size);
    const body = [
      `  <rect width="${size}" height="${size}" rx="${rx}" fill="url(#st)"/>`,
      `  <rect x="0.5" y="0.5" width="${size - 1}" height="${size - 1}" rx="${rx - 0.5}" fill="none" stroke="${t.edge[0]}" stroke-opacity="${t.edge[1]}"/>`,
      t.lit ? `  <rect x="1.5" y="1" width="${size - 3}" height="1" fill="#fff" fill-opacity="${t.lit}"/>` : '',
      `  <g shape-rendering="crispEdges">${cells}</g>`,
    ].filter(Boolean).join('\n');
    return svg({ w: size, h: size, title: label, defs, body });
  }
  const big = size >= 96;
  const inset = big ? 5 : 2;
  const TW = size - 2 * inset;
  const tile = squircle(inset, inset, TW, TW, Math.round(0.22 * TW));
  const defs = [
    onyx ? tileGrad('tile', look.tile) : vGrad('tile', look.tile.map((c, i) => [[0, 0.55, 1][i], c]), inset, inset + TW),
    `    <clipPath id="tc"><path d="${tile}"/></clipPath>`,
    onyx
      ? vGrad('rim', [[0, WHITE, 0.24], [1, WHITE, 0]], inset, inset + TW * 0.4)
      : vGrad('rim', [[0, '#000', 0], [1, '#000', 0.1]], inset + TW * 0.6, inset + TW),
  ].join('\n');
  const body = [
    `  <path d="${tile}" fill="url(#tile)"/>`,
    `  <g clip-path="url(#tc)"><path d="${tile}" fill="none" stroke="url(#rim)" stroke-width="2"/></g>`,
    onyx
      ? `  <path d="${tile}" fill="none" stroke="#fff" stroke-opacity="0.12" stroke-width="1" transform="translate(0.5 0.5) scale(${fmt((size - 1) / size)})"/>`
      : `  <path d="${tile}" fill="none" stroke="#000" stroke-opacity="0.18" stroke-width="1"/>`,
    big ? `  <g>${cells.replaceAll('/>', ' rx="1"/>')}</g>` : `  <g shape-rendering="crispEdges">${cells}</g>`,
  ].join('\n');
  return svg({ w: size, h: size, title: label, defs, body });
}

/** The native design for an icon of `size` px: Onyx, or its Paper twin for dark backgrounds. */
function iconAt(size, mode = 'onyx') {
  if (SMALL_ARROW[size]) return smallSvg(size, mode);
  return masterSvg(size, { mode, title: mode === 'paper' ? 'Boltget (for dark backgrounds)' : 'Boltget' });
}
/** The bare arrow at `size` px in one colour: the tray glyph. */
function bareAt(size, fill) {
  const body = `  <g fill="${fill}" shape-rendering="crispEdges">${pixelRects(pixelCells(size, BARE_ARROW[size]))}</g>`;
  return svg({ w: size, h: size, title: `Boltget (${size} px)`, body });
}
/** Sizes with a native design (written as svg/icon-N.svg and svg/icon-on-dark-N.svg). */
const NATIVE = [16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 128];
/** Windows' full icon set (ico/app.ico, ico/app-on-dark.ico). */
const ICO_SIZES = [16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256];
/** The tray glyph's sizes (100% to 200% scaling). */
const TRAY_SIZES = [16, 20, 24, 32];

function buildIcons() {
  writeSvg('icon.svg', masterSvg(1024));
  writeSvg('icon-animated.svg', masterSvg(1024, { animated: true, title: 'Boltget, animated' }));
  writeSvg('icon-on-dark.svg', masterSvg(1024, { mode: 'paper', title: 'Boltget (for dark backgrounds)' }));
  for (const s of NATIVE) {
    writeSvg(`svg/icon-${s}.svg`, iconAt(s));
    writeSvg(`svg/icon-on-dark-${s}.svg`, iconAt(s, 'paper'));
  }
  writeSvg('svg/icon-square.svg', masterSvg(1024, { bleed: true, title: 'Boltget (square)' }));
  writeSvg('svg/icon-maskable.svg', masterSvg(1024, { bleed: true, title: 'Boltget (maskable)' }));
  for (const s of TRAY_SIZES) {
    writeSvg(`svg/tray-ink-${s}.svg`, bareAt(s, INK));
    writeSvg(`svg/tray-white-${s}.svg`, bareAt(s, WHITE));
  }
}

/* ------------------------------------------------------------------------ */
/* 8. The bare mark, wordmark and lockups                                    */
/* ------------------------------------------------------------------------ */

const MARK_VIEWBOX = `${GRID.origin} ${GRID.origin} ${GRID.n * U} ${GRID.n * U}`;
/** The mark in ink on light, white on dark (an SVG used as an image follows the OS scheme). */
const SCHEME_FILL = (sel) => `${sel}{fill:${INK}}@media (prefers-color-scheme:dark){${sel}{fill:${WHITE}}}`;

function buildLogos() {
  const d = markPath();
  const markSvg = (title, fill) =>
    svg({ w: 108, h: 108, viewBox: MARK_VIEWBOX, title, body: `  <path fill="${fill}" d="${d}"/>` });

  writeSvg('svg/mark.svg', markSvg('Boltget mark, white (for dark backgrounds)', WHITE));
  writeSvg('svg/mark-ink.svg', markSvg('Boltget mark, ink (for light backgrounds)', INK));
  writeSvg('svg/mark-white.svg', markSvg('Boltget mark, white', WHITE));
  writeSvg('svg/mark-black.svg', markSvg('Boltget mark, black', '#000000'));

  // Animated mark (transparent), CSS-driven so it honours prefers-reduced-motion.
  const kf = MOTION.keyTimes.map((t, i) => `${fmt(t * 100)}%{opacity:${MOTION.values[i]}}`).join('');
  const rowsSvg = ROWS.map((_, r) => {
    const cells = CELLS.filter((c) => c.r === r);
    return `  <path class="r" style="animation-delay:${fmt(r * MOTION.stagger)}s" d="${markPath(GRID, cells)}"/>`;
  }).join('\n');
  writeSvg(
    'svg/mark-animated.svg',
    svg({
      w: 108, h: 108, viewBox: MARK_VIEWBOX, title: 'Boltget mark, animated',
      style: `${SCHEME_FILL('.r')}.r{opacity:${MOTION.values[0]};animation:arrive ${MOTION.dur}s linear infinite}@keyframes arrive{${kf}}@media (prefers-reduced-motion:reduce){.r{animation:none;opacity:1}}`,
      body: rowsSvg,
    }),
  );

  // Favicon: the bare mark in one colour, ink on light browser chrome, white
  // on dark. At 32 device pixels (a 2x tab) it is the 9x9 arrow on whole
  // pixels; at 16 (max-width 24px) the 5x5 arrow, 2 px cells with 1 px gaps,
  // drawn at twice the size.
  {
    const big = pixelRects(pixelCells(32, BARE_ARROW[32]));
    const small = pixelRects(pixelCells(16, BARE_ARROW[16]).map(({ x, y, w, h }) => ({ x: 2 * x, y: 2 * y, w: 2 * w, h: 2 * h })));
    writeSvg(
      'web/favicon.svg',
      svg({
        w: 32, h: 32, title: 'Boltget',
        style: SCHEME_FILL('g') + '.s{display:none}@media (max-width:24px){.b{display:none}.s{display:inline}}',
        body: [`  <g class="b" shape-rendering="crispEdges">${big}</g>`, `  <g class="s" shape-rendering="crispEdges">${small}</g>`].join('\n'),
      }),
    );
  }

  // Wordmark + lockups
  const WM_SIZE = 100;
  const wm = wordmarkText(WM_SIZE);
  const bb = wm.bbox;
  const wmBox = `${fmt(bb.x1)} ${fmt(bb.y1)} ${fmt(bb.x2 - bb.x1)} ${fmt(bb.y2 - bb.y1)}`;
  for (const [variant, fill] of [['on-dark', WHITE], ['on-light', INK]]) {
    writeSvg(
      `svg/wordmark-${variant}.svg`,
      svg({ w: fmt(bb.x2 - bb.x1), h: fmt(bb.y2 - bb.y1), viewBox: wmBox, title: 'Boltget', body: `  <path fill="${fill}" d="${wm.d}"/>` }),
    );
  }

  /*
   * Lockup proportions, in grid units u (= pitch at artboard scale, 12):
   *   arrow ink  = 106 (9u - 2)        cap height = 4u (48)
   *   gap        = 2.5u between arrow ink and wordmark ink
   *   cap block centred on the arrow ink's vertical centre
   */
  const variants = [['on-dark', WHITE, '', WHITE], ['on-light', INK, '', INK]];
  const capU = 4;
  const wmSize = (capU * U) / capHeight(SANS(WORDMARK.weight), 1);
  const ink = { x1: 37, y1: 37, x2: 143, y2: 143 };
  const midY = (ink.y1 + ink.y2) / 2;
  {
    const baseline = midY + (capU * U) / 2;
    const t = wordmarkText(wmSize, 0, 0);
    const x = ink.x2 + 2.5 * U - t.bbox.x1;
    const w2 = wordmarkText(wmSize, x, baseline);
    const vb = [ink.x1, ink.y1, w2.bbox.x2 - ink.x1, ink.y2 - ink.y1];
    for (const [variant, markFill, defs, fill] of variants) {
      writeSvg(
        `svg/lockup-horizontal-${variant}.svg`,
        svg({
          w: fmt(vb[2]), h: fmt(vb[3]), viewBox: vb.map(fmt).join(' '), title: 'Boltget', defs,
          body: `  <path fill="${markFill}" d="${d}"/>\n  <path fill="${fill}" d="${w2.d}"/>`,
        }),
      );
    }
  }
  {
    // Stacked: wordmark cap height 3u, 2.5u below the arrow ink, centred.
    const cap = 3 * U;
    const size = cap / capHeight(SANS(WORDMARK.weight), 1);
    const baseline = ink.y2 + 2.5 * U + cap;
    const w3 = wordmarkText(size, ART / 2, baseline, 'middle');
    const shift = ART / 2 - (w3.bbox.x1 + w3.bbox.x2) / 2;
    const w4 = wordmarkText(size, ART / 2 + shift, baseline, 'middle');
    const x1 = Math.min(ink.x1, w4.bbox.x1), x2 = Math.max(ink.x2, w4.bbox.x2);
    const vb = [x1, ink.y1, x2 - x1, baseline - ink.y1];
    for (const [variant, markFill, defs, fill] of variants) {
      writeSvg(
        `svg/lockup-stacked-${variant}.svg`,
        svg({
          w: fmt(vb[2]), h: fmt(vb[3]), viewBox: vb.map(fmt).join(' '), title: 'Boltget', defs,
          body: `  <path fill="${markFill}" d="${d}"/>\n  <path fill="${fill}" d="${w4.d}"/>`,
        }),
      );
    }
  }

  /*
   * The website header's lockup, pixel-snapped for the screen: 28 CSS px tall,
   * the arrow 9 cells on a 3 px pitch from the top-left, 2.5 px cells with
   * 0.5 px gaps (the big mark's proportion): crisp 1 device pixel seams at 2x,
   * soft hairline seams at 1x. Both files are kept for <img srcset>. Caps of the wordmark are 4u
   * (12 px), centred on the arrow, 2.5u after it.
   */
  {
    const H = 28, pitch = 3;
    const cap = 4 * pitch;
    const size = cap / capHeight(SANS(WORDMARK.weight), 1);
    const arrowH = 9 * pitch;
    const baseline = arrowH / 2 + cap / 2;
    const t = wordmarkText(size, 0, 0);
    const w = wordmarkText(size, arrowH + 2.5 * pitch - t.bbox.x1, baseline);
    const W = Math.ceil(w.bbox.x2);
    for (const [variant, fill] of [['on-light', INK], ['on-dark', WHITE]]) {
      for (const [suffix, px] of [['', 2.5], ['@2x', 2.5]]) {
        const rects = CELLS.map(({ r, c }) => `<rect x="${c * pitch}" y="${r * pitch}" width="${px}" height="${px}" fill="${fill}"/>`).join('');
        writeSvg(
          `svg/lockup-header-${variant}${suffix}.svg`,
          svg({ w: W, h: H, title: 'Boltget', body: `  ${rects}\n  <path fill="${fill}" d="${w.d}"/>` }),
        );
      }
    }
  }

  /*
   * Tile marks: the one-colour mark on a simple tile, for the website's
   * illustrations (the installer window on /download): the two icon modes
   * without their lighting. Light: ink cells on a white tile with a soft
   * top-to-bottom sheen and a hairline (Paper). Dark: white cells on the
   * Onyx tile. Drawn on a 40 px grid with the
   * header's proportions (3 px pitch, 2.5 px cells), so they scale cleanly.
   */
  {
    const S = 40, pitch = 3, px = 2.5, off = (S - (8 * pitch + px)) / 2, rx = 9;
    const cells = (fill) => CELLS.map(({ r, c }) => `<rect x="${off + c * pitch}" y="${off + r * pitch}" width="${px}" height="${px}" rx="0.4" fill="${fill}"/>`).join('');
    const variants = [
      ['light', 'url(#t)', WHITE, GRAPHITE[100], INK, 'rgb(10 10 11 / 0.10)'],
      ['dark', 'url(#t)', ICON.onyx.tile[0], INK, WHITE, 'rgb(255 255 255 / 0.12)'],
    ];
    for (const [name, fill, top, bottom, mark, edge] of variants) {
      const defs = `<linearGradient id="t" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${top}"/><stop offset="1" stop-color="${bottom}"/></linearGradient>`;
      writeSvg(
        `svg/tile-mark-${name}.svg`,
        svg({
          w: S, h: S, title: 'Boltget', defs,
          body: [
            `  <rect width="${S}" height="${S}" rx="${rx}" fill="${fill}"/>`,
            `  <rect x="0.5" y="0.5" width="${S - 1}" height="${S - 1}" rx="${rx - 0.5}" fill="none" stroke="${edge}"/>`,
            `  ${cells(mark)}`,
          ].join('\n'),
        }),
      );
    }
  }
}

/* ------------------------------------------------------------------------ */
/* 9. Construction drawing                                                   */
/* ------------------------------------------------------------------------ */

function buildConstruction() {
  const S = 540 / MASTER.size; // drawing scale
  const W = 900, H = 860;
  const T = { x: 180, y: 150 };
  const X = (v) => fmt(T.x + v * S), Y = (v) => fmt(T.y + v * S);
  const line = GRAPHITE[700], faint = GRAPHITE[800], label = GRAPHITE[400], acc = WHITE;
  const mono = MONO('Regular');
  const L = (str, x, y, anchor = 'middle', fill = label, size = 13) => `  <path fill="${fill}" d="${text(str, { f: mono, size, x, y, anchor, decimals: 1 }).d}"/>`;
  const { inset, pitch, px, corner, radius } = MASTER;
  const O = (MASTER.size - (8 * pitch + px)) / 2;
  const parts = [];
  parts.push(`  <rect width="${W}" height="${H}" fill="${INK}"/>`);
  // artboard and tile
  parts.push(`  <rect x="${X(0)}" y="${Y(0)}" width="${fmt(MASTER.size * S)}" height="${fmt(MASTER.size * S)}" fill="none" stroke="${GRAPHITE[600]}" stroke-dasharray="4 6"/>`);
  parts.push(`  <path d="${squircle(X(inset), Y(inset), (MASTER.size - 2 * inset) * S, (MASTER.size - 2 * inset) * S, radius * S)}" fill="${GRAPHITE[950]}" stroke="${line}"/>`);
  // grid
  let gl = '';
  for (let i = 0; i <= 9; i++) {
    const p = O + i * pitch - (i === 9 ? pitch - px : 0);
    gl += `M${X(p)} ${Y(O)}V${Y(O + 8 * pitch + px)}M${X(O)} ${Y(p)}H${X(O + 8 * pitch + px)}`;
  }
  parts.push(`  <path fill="none" stroke="${faint}" d="${gl}"/>`);
  parts.push(`  <path fill="none" stroke="${line}" stroke-dasharray="2 4" d="M${X(512)} ${Y(0)}V${Y(1024)}M${X(0)} ${Y(512)}H${X(1024)}"/>`);
  // pixels
  parts.push(`  <g fill="${WHITE}">${CELLS.map(({ r, c }) => `<rect x="${X(O + c * pitch)}" y="${Y(O + r * pitch)}" width="${fmt(px * S)}" height="${fmt(px * S)}" rx="${fmt(corner * S)}"/>`).join('')}</g>`);
  for (let i = 0; i < 9; i++) {
    parts.push(L(String(i), X(O + i * pitch + px / 2), Y(O) - 10));
    parts.push(L(String(i), X(O) - 12, Y(O + i * pitch + px / 2) + 4, 'end'));
  }
  const dimH = (x1, x2, y, str, tick = 6) =>
    `  <path fill="none" stroke="${label}" d="M${x1} ${y}H${x2}M${x1} ${y - tick}V${y + tick}M${x2} ${y - tick}V${y + tick}"/>\n` + L(str, (x1 + x2) / 2, y - 10);
  const dimV = (y1, y2, x, str, side = 'start') =>
    `  <path fill="none" stroke="${label}" d="M${x} ${y1}V${y2}M${x - 6} ${y1}H${x + 6}M${x - 6} ${y2}H${x + 6}"/>\n` + L(str, side === 'end' ? x - 12 : x + 12, (y1 + y2) / 2 + 4, side);
  parts.push(dimH(X(0), X(1024), Y(0) - 44, '1024'));
  const yb = Y(1024) + 40;
  parts.push(dimH(X(0), X(inset), yb, String(inset)));
  parts.push(dimH(X(inset), X(1024 - inset), yb, `tile ${1024 - 2 * inset}`));
  parts.push(dimH(X(O), X(O + 8 * pitch + px), Y(O + 8 * pitch + px) + 24, `${8 * pitch + px}`));
  const xr = X(1024) + 30;
  parts.push(dimV(Y(O), Y(O + pitch), xr, `pitch ${pitch}`));
  parts.push(dimV(Y(O), Y(O + px), xr + 120, `pixel ${px}`));
  parts.push(L(`r = ${radius}, smoothing ${MASTER.smoothing}`, X(inset), Y(inset) - 14, 'start', acc));
  parts.push(L(`Artboard 1024 · tile ${1024 - 2 * inset} squircle · grid 9 × 9 · pitch ${pitch} · pixel ${px} (gap ${pitch - px}, corner ${corner}) · Onyx: tile ${ICON.onyx.tile.join(' → ')}, arrow white`, 40, H - 34, 'start', GRAPHITE[500], 12));
  writeSvg(
    'svg/construction.svg',
    svg({ w: W, h: H, title: 'Boltget icon construction', body: parts.join('\n') }),
  );
}

/* ------------------------------------------------------------------------ */
/* 10. Social images                                                         */
/* ------------------------------------------------------------------------ */

/**
 * The Open Graph art (BRAND.md, "Source art"): a generated raster, 1200 x
 * 630, with the headline, a large Onyx icon in the clouds and a small icon
 * and wordmark top-left. Generated art cannot draw the 9x9 grid exactly at
 * 50 px, so the small icon is covered with the real one.
 */
const OG_ART = {
  file: join(ROOT, 'source', 'og-art.png'),
  w: 1200, h: 630,
  /** The small icon's tile in the art: centre and side, in art pixels. */
  icon: { cx: 89, cy: 87, tile: 51 },
  /** The ground around the small icon, sampled to cover it. */
  ground: { left: 40, top: 40, size: 100 },
};

/** The mean colour of the art's ground around the small icon, as #RRGGBB. */
async function ogGround() {
  const { left, top, size } = OG_ART.ground;
  const { data, info } = await sharp(OG_ART.file).extract({ left, top, width: size, height: size }).removeAlpha().raw().toBuffer({ resolveWithObject: true });
  const sum = [0, 0, 0];
  let n = 0;
  for (let y = 0; y < info.height; y++) {
    for (let x = 0; x < info.width; x++) {
      // Only the frame outside the icon and its shadow.
      if (x >= 16 && x < 84 && y >= 16 && y < 84) continue;
      const i = (y * info.width + x) * 3;
      for (let c = 0; c < 3; c++) sum[c] += data[i + c];
      n++;
    }
  }
  return toHex(sum.map((v) => v / n));
}

/**
 * The OG card at W x H: the art, scaled to cover and centred, with the real
 * Onyx icon over the art's small one (on a soft patch of the art's own ground
 * that hides the generated icon's edge and shadow).
 */
async function ogSvg(W, H, title) {
  const k = Math.max(W / OG_ART.w, H / OG_ART.h);
  const aw = OG_ART.w * k, ah = OG_ART.h * k;
  const ax = (W - aw) / 2, ay = (H - ah) / 2;
  const { cx, cy, tile } = OG_ART.icon;
  // The small-size design (a tile inset 2 px) covering the art's tile, with
  // the 9x9 arrow at about the art's proportion (two thirds of the tile).
  const size = Math.round(tile * k) + 4;
  const pitch = Math.round(((size - 4) * 0.68) / 9);
  const x = Math.round(ax + cx * k - size / 2), y = Math.round(ay + cy * k - size / 2);
  const patch = Math.round(size * 1.12);
  const art = await sharp(OG_ART.file).png().toBuffer();
  const body = [
    `  <image x="${fmt(ax)}" y="${fmt(ay)}" width="${fmt(aw)}" height="${fmt(ah)}" preserveAspectRatio="none" href="${dataUri(art)}"/>`,
    `  <rect x="${fmt(x + size / 2 - patch / 2)}" y="${fmt(y + size / 2 - patch / 2 + 2 * k)}" width="${patch}" height="${patch}" rx="${fmt(patch * 0.28)}" fill="${await ogGround()}" filter="url(#soft)"/>`,
    `  <image x="${x}" y="${y}" width="${size}" height="${size}" href="${dataUri(render(smallSvg(size, 'onyx', [9, pitch - 1, 1]), size).asPng())}"/>`,
  ].join('\n');
  const defs = `    <filter id="soft" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="${fmt(2 * k)}"/></filter>`;
  return svg({ w: W, h: H, title, defs, body });
}

/** The sky photograph behind the X header, cropped to W x H (resvg cannot decode webp). */
async function skyPng(W, H) {
  return sharp(join(REPO, 'apps', 'web', 'src', 'assets', 'sky.webp')).resize(W, H, { fit: 'cover', position: 'top' }).png().toBuffer();
}

/**
 * X header (1500 x 500): the pearl sky, the headline on one line and the
 * subline in ink, centred in the band X keeps clear of the avatar (lower
 * left) and the profile buttons (lower right).
 */
async function xHeaderSvg(W, H) {
  const P = [`<image width="${W}" height="${H}" href="${dataUri(await skyPng(W, H))}"/>`];
  const a = text(COPY.headline[0] + ' ', { f: SANS('SemiBold'), size: 64, tracking: -0.035 });
  const b = text(COPY.headline[1], { f: SERIF(true), size: 70, tracking: -0.012 });
  const x = W / 2 + 60 - (a.width + b.width) / 2;
  P.push(`<path fill="${INK}" d="${text(COPY.headline[0], { f: SANS('SemiBold'), size: 64, x, y: 230, tracking: -0.035 }).d}"/>`);
  P.push(`<path fill="${INK}" d="${text(COPY.headline[1], { f: SERIF(true), size: 70, x: x + a.width, y: 230, tracking: -0.012 }).d}"/>`);
  P.push(`<path fill="${INK}" fill-opacity="0.7" d="${text(COPY.subline, { f: SERIF(false), size: 26, x: W / 2 + 60, y: 290, anchor: 'middle' }).d}"/>`);
  return svg({ w: W, h: H, title: 'Boltget — every byte, at full speed', body: P.join('\n') });
}

/** Writes a large social PNG, compressed. */
async function writeSocialPng(rel, svgText, w, h) {
  const png = render(svgText, w, h).asPng();
  write(out(rel), await sharp(png).png({ compressionLevel: 9, adaptiveFiltering: true }).toBuffer());
}

async function buildSocial() {
  await writeSocialPng('social/og-image.png', await ogSvg(1200, 630, 'Boltget — every byte, at full speed'), 1200, 630);
  // GitHub social preview 1280 x 640: the same card, scaled to cover.
  await writeSocialPng('social/github-social.png', await ogSvg(1280, 640, 'Boltget — every byte, at full speed'), 1280, 640);
  await writeSocialPng('social/x-header.png', await xHeaderSvg(1500, 500), 1500, 500);

  // Avatar: full bleed (platforms crop to a circle).
  const avatar = masterSvg(1024, { bleed: true, title: 'Boltget avatar' });
  writeSvg('social/avatar.svg', avatar);
  writePng('social/avatar-400.png', masterSvg(400, { bleed: true }), 400, 400);
  writePng('social/avatar-512.png', masterSvg(512, { bleed: true }), 512, 512);
}

/* ------------------------------------------------------------------------ */
/* 11. Rasters + ICO                                                         */
/* ------------------------------------------------------------------------ */

function buildRasters() {
  const rendered = {};
  const onDark = {};
  for (const s of [...NATIVE, 256, 512, 1024]) {
    rendered[s] = writePng(`png/icon-${s}.png`, iconAt(s), s, s);
    onDark[s] = writePng(`png/icon-on-dark-${s}.png`, iconAt(s, 'paper'), s, s);
  }
  const tray = { ink: {}, white: {} };
  for (const s of TRAY_SIZES) {
    tray.ink[s] = writePng(`png/tray-ink-${s}.png`, bareAt(s, INK), s, s);
    tray.white[s] = writePng(`png/tray-white-${s}.png`, bareAt(s, WHITE), s, s);
  }

  // Lockups as PNG (for slides/docs) at a fixed height of 240 px.
  for (const n of ['lockup-horizontal-on-dark', 'lockup-horizontal-on-light', 'lockup-stacked-on-dark', 'lockup-stacked-on-light']) {
    const src = svgs[`svg/${n}.svg`];
    const [, , vw, vh] = src.match(/viewBox="([^"]+)"/)[1].split(' ').map(Number);
    writePng(`png/${n}.png`, src, Math.round((vw / vh) * 240));
  }

  // Web
  writePng('web/apple-touch-icon.png', masterSvg(180, { bleed: true }), 180, 180);
  writePng('web/icon-192.png', masterSvg(192), 192, 192);
  writePng('web/icon-512.png', masterSvg(512), 512, 512);
  // Maskable: full bleed; the arrow stays well inside the central 80% safe zone.
  writePng('web/icon-maskable-512.png', masterSvg(512, { bleed: true }), 512, 512);
  write(out('web/favicon.ico'), encodeIco([16, 32, 48].map((size) => ({ size, rendered: rendered[size] }))));
  write(
    out('web/site.webmanifest'),
    JSON.stringify(
      {
        name: 'Boltget',
        short_name: 'Boltget',
        icons: [
          { src: '/icon-192.png', sizes: '192x192', type: 'image/png' },
          { src: '/icon-512.png', sizes: '512x512', type: 'image/png' },
          { src: '/icon-maskable-512.png', sizes: '512x512', type: 'image/png', purpose: 'maskable' },
        ],
        theme_color: INK,
        background_color: INK,
        display: 'standalone',
      },
      null,
      2,
    ) + '\n',
  );

  // Windows app icon: the full set Windows asks for, every size native.
  // Onyx is the default; the Paper twin is for a dark taskbar.
  write(out('ico/app.ico'), encodeIco(ICO_SIZES.map((size) => ({ size, rendered: rendered[size] }))));
  write(out('ico/app-on-dark.ico'), encodeIco(ICO_SIZES.map((size) => ({ size, rendered: onDark[size] }))));
  // Tray glyphs: ink for a light taskbar, white for a dark one.
  for (const c of ['ink', 'white']) {
    write(out(`ico/tray-${c}.ico`), encodeIco(TRAY_SIZES.map((size) => ({ size, rendered: tray[c][size] }))));
  }
}

/** The maskable icon's arrow must sit inside the central 80% (a circle of radius 40%). */
function checkMaskable() {
  const k = MASTER.size / (MASTER.size - 2 * MASTER.inset);
  const half = ((8 * MASTER.pitch + MASTER.px) * k) / 2;
  const cell = MASTER.pitch * k;
  // The farthest corners: the head's widest row (at the centre row) and the shaft's top corners.
  const far = Math.max(Math.hypot(half, cell / 2), Math.hypot(1.5 * cell, half));
  if (far > 0.4 * MASTER.size) throw new Error('maskable icon: the arrow leaves the safe zone');
}

/* ------------------------------------------------------------------------ */
/* 12. Tokens + fonts                                                        */
/* ------------------------------------------------------------------------ */

function buildTokens() {
  const tokens = {
    ink: INK,
    paper: PAPER,
    white: WHITE,
    graphite: GRAPHITE,
    semantic: SEMANTIC,
    theme: THEME,
    mark: { ink: INK, white: WHITE, onyx: ICON.onyx.tile, paper: ICON.paper.tile },
    gradient: { progress: PROGRESS_DARK, progressLight: PROGRESS_LIGHT },
    atmosphere: ATMOSPHERE,
    font: FONT,
    motion: MOTION,
  };
  write(out('tokens/tokens.json'), JSON.stringify(tokens, null, 2) + '\n');
  const v = (o, p) => Object.entries(o).map(([k, c]) => `  --${p}-${k}: ${c};`).join('\n');
  const theme = (t) => [
    ...Object.entries(THEME[t]).map(([k, c]) => `  --bg-${k}: ${c};`.replace('--bg-bg', '--bg')),
    ...Object.entries(SEMANTIC).map(([k, c]) => `  --${k}: ${c[t]};`),
  ].join('\n');
  const css = `/* Boltget design tokens — generated by scripts/build.mjs */
:root {
  --ink: ${INK};
  --paper: ${PAPER};
  --white: ${WHITE};
${v(GRAPHITE, 'graphite')}
${v(ATMOSPHERE, 'atmosphere')}
  --mark-ink: ${INK};
  --gradient-tile: linear-gradient(120deg, ${ICON.onyx.tile.join(', ')});
  --gradient-tile-paper: linear-gradient(180deg, ${ICON.paper.tile.join(', ')});
  --gradient-progress: linear-gradient(90deg, ${PROGRESS_DARK.map(([o, c]) => `${c} ${o * 100}%`).join(', ')});
  --gradient-progress-light: linear-gradient(90deg, ${PROGRESS_LIGHT.map(([o, c]) => `${c} ${o * 100}%`).join(', ')});
  --font-display: "${FONT.display.family}", ui-sans-serif, system-ui, sans-serif;
  --font-sans: "${FONT.text.family}", ui-sans-serif, system-ui, sans-serif;
  --font-mono: "${FONT.mono.family}", ui-monospace, "SFMono-Regular", Menlo, Consolas, monospace;
  --font-display-weight: ${FONT.display.weight};
  --font-display-tracking: ${FONT.display.tracking}em;
  --font-text-weight: ${FONT.text.weight};
  --motion-arrive-duration: ${MOTION.dur}s;
  --motion-arrive-stagger: ${MOTION.stagger}s;
}
/* Semantic theme tokens. Dark is the default; light is opt-in. */
:root, [data-theme="dark"] {
${theme('dark')}
}
[data-theme="light"] {
${theme('light')}
}
`.replace(/--bg-(surface|raised|border|border-strong|text|text-muted|text-subtle|accent|accent-hover|on-accent|accent-subtle):/g, '--$1:');
  write(out('tokens/tokens.css'), css);
}

function copyFonts() {
  mkdirSync(out('fonts'), { recursive: true });
  const files = [
    // Red Hat Display and Red Hat Text: variable latin subsets, vendored in source/fonts.
    [join(SOURCE_FONTS, 'RedHatDisplay-latin.woff2'), 'RedHatDisplay-latin.woff2'],
    [join(SOURCE_FONTS, 'RedHatText-latin.woff2'), 'RedHatText-latin.woff2'],
    [join(SOURCE_FONTS, 'RedHat-OFL.txt'), 'RedHat-OFL.txt'],
    // Geist Mono, from the geist package.
    [join(FONT_DIR, 'geist-mono/GeistMono-Regular.woff2'), 'GeistMono-Regular.woff2'],
    [join(FONT_DIR, 'geist-mono/GeistMono-Medium.woff2'), 'GeistMono-Medium.woff2'],
    [join(ROOT, 'node_modules', 'geist', 'LICENSE.TXT'), 'Geist-OFL.txt'],
  ];
  for (const [from, to] of files) {
    copyFileSync(from, out('fonts', to));
    written.push(`fonts/${to}`);
  }
}

/* ------------------------------------------------------------------------ */
/* 13. preview.html                                                          */
/* ------------------------------------------------------------------------ */

function buildPreview() {
  const swatch = (name, c) => `<div class="sw"><span style="background:${c}"></span><b>${name}</b><code>${c}</code></div>`;
  const scale = (title, obj, prefix) =>
    `<h3>${title}</h3><div class="swatches">${Object.entries(obj).map(([k, c]) => swatch(`${prefix}-${k}`, c)).join('')}</div>`;
  const icon = (mode) => (mode === 'paper' ? 'png/icon-on-dark' : 'png/icon');
  const iconRow = (mode) => [...NATIVE, 256].map((s) => `<figure><img src="${icon(mode)}-${s}.png" width="${s}" height="${s}" alt=""><figcaption>${s}</figcaption></figure>`).join('');
  const zoomRow = (mode) => NATIVE.filter((s) => s <= 64).map((s) => `<figure><img class="px" src="${icon(mode)}-${s}.png" width="${s * (s > 32 ? 2 : 4)}" height="${s * (s > 32 ? 2 : 4)}" alt=""><figcaption>${s} @${s > 32 ? 2 : 4}×</figcaption></figure>`).join('');
  const trayRow = (t) => TRAY_SIZES.map((s) => `<figure><img class="px" src="png/tray-${t === 'dark' ? 'white' : 'ink'}-${s}.png" width="${s * 2}" height="${s * 2}" alt=""><figcaption>${s} @2×</figcaption></figure>`).join('');
  const both = (inner) => `<div class="duo"><div class="on-dark">${inner('dark')}</div><div class="on-light">${inner('light')}</div></div>`;
  const theme = (t) =>
    `<div class="theme ${t}"><h4>${t === 'dark' ? 'Dark theme' : 'Light theme'}</h4>${Object.entries(THEME[t]).map(([k, c]) => swatch(k, c)).join('')}${Object.entries(SEMANTIC).map(([k, c]) => swatch(k, c[t])).join('')}</div>`;

  const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Boltget brand kit</title>
<link rel="icon" href="web/favicon.svg" type="image/svg+xml">
<style>
@font-face{font-family:"Red Hat Display";src:url(fonts/RedHatDisplay-latin.woff2) format("woff2");font-weight:300 900}
@font-face{font-family:"Red Hat Text";src:url(fonts/RedHatText-latin.woff2) format("woff2");font-weight:300 700}
@font-face{font-family:"Geist Mono";src:url(fonts/GeistMono-Regular.woff2) format("woff2");font-weight:400}
:root{--bg:${INK};--surface:${GRAPHITE[950]};--border:${GRAPHITE[800]};--text:${GRAPHITE[50]};--muted:${GRAPHITE[400]};--subtle:${GRAPHITE[500]};--accent:${WHITE}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--text);font:450 15px/1.55 "Red Hat Text",system-ui,sans-serif;-webkit-font-smoothing:antialiased}
main{max-width:1180px;margin:0 auto;padding:56px 24px 96px}
header{display:flex;align-items:center;gap:20px;margin-bottom:56px}
header img{width:64px;height:64px}
h1{font-family:"Red Hat Display",system-ui,sans-serif;font-weight:600;font-size:28px;letter-spacing:-.02em;margin:0}
header p{margin:2px 0 0;color:var(--muted)}
h2{font-weight:600;font-size:13px;letter-spacing:.12em;text-transform:uppercase;color:var(--accent);font-family:"Geist Mono",monospace;margin:72px 0 20px;padding-top:24px;border-top:1px solid var(--border)}
h3{font-weight:500;font-size:15px;margin:28px 0 12px;color:var(--muted)}
h4{margin:0 0 12px;font-weight:500}
code{font-family:"Geist Mono",monospace;font-size:12px}
.duo{display:grid;grid-template-columns:1fr 1fr;border-radius:16px;overflow:hidden;border:1px solid var(--border)}
.duo>div{padding:32px;display:flex;flex-wrap:wrap;align-items:flex-end;justify-content:center;gap:24px;min-height:160px}
.on-dark{background:#1C1C1E}
.on-light{background:#ECEEF1;color:${INK}}
figure{margin:0;display:flex;flex-direction:column;align-items:center;gap:8px}
figcaption{font:12px "Geist Mono",monospace;color:var(--subtle)}
.px{image-rendering:pixelated}
.social{display:grid;gap:24px}
.social img{width:100%;border-radius:10px;border:1px solid var(--border);display:block}
.social figure{align-items:stretch}
.swatches{display:grid;grid-template-columns:repeat(auto-fill,minmax(150px,1fr));gap:10px}
.sw{display:grid;grid-template-columns:36px 1fr;grid-template-rows:auto auto;column-gap:10px;align-items:center;font-size:13px}
.sw span{grid-row:span 2;width:36px;height:36px;border-radius:8px;border:1px solid #ffffff1a}
.sw b{font-weight:500}.sw code{color:var(--subtle)}
.themes{display:grid;grid-template-columns:1fr 1fr;gap:16px}
.theme{border-radius:14px;padding:20px;display:grid;grid-template-columns:repeat(auto-fill,minmax(150px,1fr));gap:10px;border:1px solid var(--border)}
.theme h4{grid-column:1/-1}
.theme.dark{background:${INK}}
.theme.light{background:${WHITE};color:${INK}}
@media (max-width:760px){.duo,.themes{grid-template-columns:1fr}}
</style>
</head>
<body>
<main>
<header><img src="icon-animated.svg" alt=""><div><h1>Boltget brand kit</h1><p>Generated by <code>scripts/build.mjs</code>. Every asset on dark and light.</p></div></header>

<h2>App icon</h2>
${both(() => `<figure><img src="icon.svg" width="200" height="200" alt=""><figcaption>icon.svg (Onyx)</figcaption></figure><figure><img src="icon-animated.svg" width="200" height="200" alt=""><figcaption>icon-animated.svg</figcaption></figure><figure><img src="icon-on-dark.svg" width="200" height="200" alt=""><figcaption>icon-on-dark.svg (Paper)</figcaption></figure>`)}
<h3>Native sizes (16–40 flat tile, 48–128 snapped, 256+ master): Onyx, then Paper</h3>
${both(() => iconRow('onyx'))}
<div style="height:16px"></div>
${both(() => iconRow('paper'))}
<h3>Small sizes, magnified: the primary on light, the twin on dark</h3>
${both((t) => zoomRow(t === 'dark' ? 'paper' : 'onyx'))}
<h3>Tray glyphs</h3>
${both(trayRow)}
<h3>Favicon, apple-touch and maskable</h3>
${both((t) => `<figure><img class="px" src="${t === 'dark' ? 'web/favicon.svg' : 'png/icon-16.png'}" width="64" height="64" alt=""><figcaption>favicon ${t === 'dark' ? '(scheme-aware)' : '16'}</figcaption></figure><figure><img src="web/apple-touch-icon.png" width="90" height="90" style="border-radius:20px" alt=""><figcaption>apple-touch-icon</figcaption></figure><figure><img src="web/icon-maskable-512.png" width="90" height="90" style="border-radius:50%" alt=""><figcaption>maskable (circle crop)</figcaption></figure>`)}

<h2>Mark &amp; lockups</h2>
${both((t) => (t === 'dark' ? ['mark', 'mark-animated', 'mark-white'] : ['mark-ink', 'mark-black']).map((n) => `<figure><img src="svg/${n}.svg" width="96" height="96" alt=""><figcaption>${n}</figcaption></figure>`).join(''))}
<div style="height:16px"></div>
${both((t) => `<figure><img src="svg/lockup-horizontal-on-${t}.svg" height="72" alt=""><figcaption>lockup-horizontal-on-${t}</figcaption></figure><figure><img src="svg/lockup-header-on-${t}.svg" srcset="svg/lockup-header-on-${t}.svg 1x, svg/lockup-header-on-${t}@2x.svg 2x" height="28" alt=""><figcaption>lockup-header-on-${t}</figcaption></figure>`)}
<div style="height:16px"></div>
${both((t) => `<figure><img src="svg/lockup-stacked-on-${t}.svg" height="160" alt=""><figcaption>lockup-stacked-on-${t}</figcaption></figure>`)}

<h2>Construction</h2>
<img src="svg/construction.svg" style="width:100%;max-width:900px;display:block;margin:0 auto;border-radius:14px;border:1px solid var(--border)" alt="Construction drawing">

<h2>Social</h2>
<div class="social">
<figure><img src="social/og-image.png" alt=""><figcaption>og-image.png · 1200×630</figcaption></figure>
<figure><img src="social/github-social.png" alt=""><figcaption>github-social.png · 1280×640</figcaption></figure>
<figure><img src="social/x-header.png" alt=""><figcaption>x-header.png · 1500×500</figcaption></figure>
</div>
<div style="display:flex;gap:24px;margin-top:24px;align-items:flex-end">
<figure><img src="social/avatar-400.png" width="160" height="160" style="border-radius:50%" alt=""><figcaption>avatar · circle crop</figcaption></figure>
<figure><img src="social/avatar-400.png" width="40" height="40" style="border-radius:50%" alt=""><figcaption>40</figcaption></figure>
</div>

<h2>Color</h2>
<h3>Mark</h3>
<div class="swatches">${swatch('ink', INK)}${swatch('paper', PAPER)}${swatch('white', WHITE)}${ICON.onyx.tile.map((c, i) => swatch(`onyx tile ${i}`, c)).join('')}${ICON.paper.tile.map((c, i) => swatch(`paper tile ${i}`, c)).join('')}</div>
${scale('Graphite', GRAPHITE, 'graphite')}
${scale('Atmosphere (website: the pearl sky)', ATMOSPHERE, 'atmosphere')}
<h3>Themes &amp; semantic colors</h3>
<div class="themes">${theme('dark')}${theme('light')}</div>
</main>
</body>
</html>
`;
  write(out('preview.html'), html);
}

/* ------------------------------------------------------------------------ */
/* 14. Run                                                                   */
/* ------------------------------------------------------------------------ */

for (const dir of ['svg', 'png', 'ico', 'web', 'social', 'tokens', 'fonts']) rmSync(out(dir), { recursive: true, force: true });
checkMaskable();
buildIcons();
buildLogos();
buildConstruction();
await buildSocial();
buildRasters();
buildTokens();
copyFonts();
buildPreview();
console.log(`Boltget brand: wrote ${written.length} files`);
for (const f of written.sort()) console.log('  ' + f);
