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

/** Ink: the tile, the one-colour mark and the wordmark on light. */
const INK = '#13212B';
/** The tile's 120 degree gradient, light to deep. */
const TILE = ['#1B2D3A', '#13212B', '#0E1A23'];
/** The arrow's ramp, top to tip: white to sky blue. */
const RAMP = [[0, '#FFFFFF'], [0.42, '#E1F2FD'], [0.7, '#93D0F9'], [1, '#2E95E6']];
/** The soft sky light inside the tile. */
const SKY_LIGHT = '#38BDF8';

const C = { white: '#FFFFFF', black: '#0B0B0F', onDark: '#FAFAFA' };

const NEUTRAL = {
  0: '#FFFFFF', 50: '#FAFAFA', 100: '#F4F4F5', 200: '#E4E4E7', 300: '#D4D4D8',
  400: '#A1A1AA', 500: '#71717A', 600: '#52525B', 700: '#3F3F46', 800: '#27272A',
  850: '#1C1C21', 900: '#141418', 950: '#0B0B0F',
};
/** Emerald stays the app's colour for progress, focus and success. */
const EMERALD = {
  50: '#ECFDF5', 100: '#D1FAE5', 200: '#A7F3D0', 300: '#6EE7B7', 400: '#34D399',
  500: '#10B981', 600: '#059669', 700: '#047857', 800: '#065F46', 900: '#064E3B',
  950: '#022C22',
};
const SEMANTIC = {
  success: { dark: EMERALD[400], light: EMERALD[700] },
  warning: { dark: '#FB923C', light: '#C2410C' },
  error: { dark: '#F87171', light: '#DC2626' },
  info: { dark: '#7DD3FC', light: '#0369A1' },
};
const THEME = {
  dark: {
    bg: NEUTRAL[950], surface: NEUTRAL[900], raised: NEUTRAL[850], border: NEUTRAL[800],
    'border-strong': NEUTRAL[700], text: NEUTRAL[50], 'text-muted': NEUTRAL[400],
    'text-subtle': NEUTRAL[500], accent: EMERALD[400], 'accent-hover': EMERALD[300],
    'on-accent': EMERALD[950], 'accent-subtle': '#34D3991A',
  },
  light: {
    bg: NEUTRAL[0], surface: NEUTRAL[50], raised: NEUTRAL[100], border: NEUTRAL[200],
    'border-strong': NEUTRAL[300], text: NEUTRAL[950], 'text-muted': NEUTRAL[600],
    'text-subtle': NEUTRAL[500], accent: EMERALD[700], 'accent-hover': EMERALD[800],
    'on-accent': NEUTRAL[0], 'accent-subtle': '#0596691A',
  },
};

/**
 * Atmosphere (website only, BRAND.md "Atmosphere (website)"): the hero field's
 * ramp, top to bottom. Teal lives only here and in data series.
 */
const ATMOSPHERE = {
  deep: '#031E18', 'emerald-deep': '#023428', emerald: '#046C52', teal: '#0D9488',
  'teal-light': '#34D3AF', mint: EMERALD[200], mist: EMERALD[50], paper: NEUTRAL[0],
};

/** Progress fill (the app's running downloads): emerald, darker on light. */
const PROGRESS_DARK = [[0, EMERALD[200]], [0.5, EMERALD[400]], [1, EMERALD[600]]];
const PROGRESS_LIGHT = [[0, EMERALD[400]], [0.5, EMERALD[500]], [1, EMERALD[700]]];

/** Copy. */
const COPY = {
  headline: ['Every byte,', 'at full speed.'],
  subline: 'The download manager for Windows.',
  sublineLines: ['The download manager', 'for Windows.'],
};

/* ------------------------------------------------------------------------ */
/* 2. Colour and geometry helpers                                            */
/* ------------------------------------------------------------------------ */

const fmt = (n) => +n.toFixed(3);
const hex = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
const toHex = (rgb) => '#' + rgb.map((v) => Math.round(Math.max(0, Math.min(255, v))).toString(16).padStart(2, '0')).join('').toUpperCase();
const mix = (a, b, t) => toHex(hex(a).map((v, i) => v + (hex(b)[i] - v) * t));
function sample(stops, t) {
  for (let i = 1; i < stops.length; i++) {
    if (t <= stops[i][0]) return mix(stops[i - 1][1], stops[i][1], (t - stops[i - 1][0]) / (stops[i][0] - stops[i - 1][0]));
  }
  return stops.at(-1)[1];
}
/** The ramp sampled at the middle of each of the 9 rows (flat fills at small sizes). */
const ROW_COLORS = ROWS.map((_, r) => sample(RAMP, (r + 0.5) / 9));
/** The three bands of the gapless small glyphs: shaft, head, tip. */
const BAND = { s: '#FFFFFF', h: '#B5DFFB', t: RAMP.at(-1)[1] };

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
 * Pixel-snapped 9x9 grid in device pixels: integer cells and gaps. With
 * `extra`, the centre column is 1 px wider so the arrow centres on even sizes.
 */
function snappedCells(size, px, gap, { extra = 1, oy = null, ox = null } = {}) {
  const colW = (c) => px + (c === 4 ? extra : 0);
  const xs = [];
  let x = 0;
  for (let c = 0; c < 9; c++) { xs.push(x); x += colW(c) + gap; }
  const totalW = x - gap;
  const x0 = ox ?? Math.round((size - totalW) / 2);
  const y0 = oy ?? Math.floor((size - (9 * px + 8 * gap)) / 2);
  return CELLS.map(({ r, c }) => ({ r, c, x: x0 + xs[c], y: y0 + r * (px + gap), w: colW(c), h: px }));
}

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
const tileGrad = (id) => `    <linearGradient id="${id}" x1="0.067" y1="0.25" x2="0.933" y2="0.75">${stops(TILE.map((c, i) => [i / (TILE.length - 1), c]))}</linearGradient>`;

/* ------------------------------------------------------------------------ */
/* 4. Type: Geist (opentype.js) and Source Serif 4 (fontkitten), outlined    */
/* ------------------------------------------------------------------------ */

const fonts = {};
const font = (file) => (fonts[file] ??= opentype.loadSync(join(FONT_DIR, file)));
const SANS = (w) => font(`geist-sans/Geist-${w}.ttf`);
const MONO = (w) => font(`geist-mono/GeistMono-${w}.ttf`);
/** Source Serif 4 ships to the site as woff2 only; fontkitten reads it. */
const SERIF = (italic = false) =>
  (fonts[`serif-${italic}`] ??= openFont(readFileSync(join(REPO, 'apps', 'web', 'src', 'fonts', `SourceSerif4-400${italic ? '-italic' : ''}-latin.woff2`))));

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
 * The master icon, drawn natively at `size` px: the ink tile (120 degree
 * gradient, a faint sky light, a top rim highlight, a darker bottom rim and a
 * 1 px outer edge), the arrow in one continuous white-to-sky ramp, each pixel
 * with a little depth (light top edge, darker foot) on a soft contact shadow.
 * The pixel grid is snapped to whole device pixels at every size.
 *
 * bleed: fill the whole square (apple-touch, avatars, maskable); the arrow
 * keeps its size relative to the visible tile. animated: the rows light up
 * top to bottom (SMIL, the brand signature).
 */
function masterSvg(size, { bleed = false, animated = false, title = 'Boltget' } = {}) {
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
  const cell = ({ r, c }, extra = '') => `<rect x="${at(c)}" y="${at(r)}" width="${px}" height="${px}" rx="${corner}"${extra}/>`;
  const sh = fmt(Math.max(0.6, 8 * s * k)); // shadow offset
  const hl = fmt(Math.max(0.5, 2 * s * k)); // highlight edge

  const defs = [
    tileGrad('tile'),
    `    <clipPath id="tc"><path d="${tile}"/></clipPath>`,
    `    <radialGradient id="amb" gradientUnits="userSpaceOnUse" cx="${fmt(size / 2)}" cy="${fmt(inset + TW * 0.58)}" r="${fmt(TW * 0.45)}">${stops([[0, SKY_LIGHT, 0.1], [0.55, SKY_LIGHT, 0.035], [1, SKY_LIGHT, 0]])}</radialGradient>`,
    vGrad('rimT', [[0, '#FFFFFF', 0.16], [1, '#FFFFFF', 0]], inset, inset + TW * 0.4),
    vGrad('rimB', [[0, '#05090C', 0], [0.6, '#05090C', 0.35], [1, '#05090C', 0.35]], inset + TW * 0.7, inset + TW),
    `    <filter id="sh" x="0" y="0" width="${size}" height="${size}" filterUnits="userSpaceOnUse"><feGaussianBlur stdDeviation="${fmt(5 * s * k)}"/></filter>`,
    `    <linearGradient id="ov" x1="0" y1="0" x2="0" y2="1">${stops([[0, '#FFFFFF', 0.12], [0.45, '#FFFFFF', 0], [1, '#0B2235', 0.1]])}</linearGradient>`,
    vGrad('ramp', RAMP, O, O + span),
  ];
  const rowCells = (r) => CELLS.filter((c) => c.r === r);
  // Per row: its contact shadow, pixels, depth and top highlight, so a row
  // can fade as one in the animated icon.
  const rowLayer = (r) => {
    const cells = rowCells(r);
    defs.push(`    <mask id="hl${r}" maskUnits="userSpaceOnUse" x="0" y="0" width="${size}" height="${size}"><g fill="#fff">${cells.map((c) => cell(c)).join('')}</g><g fill="#000" transform="translate(0 ${hl})">${cells.map((c) => cell(c)).join('')}</g></mask>`);
    const parts = [
      `<g filter="url(#sh)" transform="translate(0 ${sh})" fill="#03080C" fill-opacity="0.55">${cells.map((c) => cell(c)).join('')}</g>`,
      `<g fill="url(#ramp)">${cells.map((c) => cell(c)).join('')}</g>`,
      `<g fill="url(#ov)">${cells.map((c) => cell(c)).join('')}</g>`,
      `<rect width="${size}" height="${size}" fill="#fff" fill-opacity="0.3" mask="url(#hl${r})"/>`,
    ].join('');
    if (!animated) return `  <g>${parts}</g>`;
    const begin = +(r * MOTION.stagger).toFixed(2);
    return `  <g opacity="${MOTION.values[0]}"><animate attributeName="opacity" values="${MOTION.values.join(';')}" keyTimes="${MOTION.keyTimes.join(';')}" dur="${MOTION.dur}s" begin="${begin}s" repeatCount="indefinite"/>${parts}</g>`;
  };
  const rows = ROWS.map((_, r) => rowLayer(r));
  const body = [
    bleed ? '' : `  <path d="${tile}" fill="none" stroke="#000" stroke-opacity="0.25" stroke-width="${fmt(2 * s)}"/>`,
    `  <path d="${tile}" fill="url(#tile)"/>`,
    `  <g clip-path="url(#tc)"><rect width="${size}" height="${size}" fill="url(#amb)"/><path d="${tile}" fill="none" stroke="url(#rimT)" stroke-width="${fmt(6 * s)}"/><path d="${tile}" fill="none" stroke="url(#rimB)" stroke-width="${fmt(4 * s)}"/></g>`,
    ...rows,
  ].filter(Boolean).join('\n');
  return svg({ w: size, h: size, title, defs: defs.join('\n'), body });
}

/**
 * Gapless stepped glyphs for 16 to 40 px, drawn in device pixels:
 * [x0, x1, y0, y1, band] per rectangle, band s(haft), h(ead) or t(ip).
 */
const STEPPED = {
  16: [[6, 10, 3, 8, 's'], [3, 13, 8, 9, 'h'], [4, 12, 9, 10, 'h'], [5, 11, 10, 11, 'h'], [6, 10, 11, 12, 't'], [7, 9, 12, 13, 't']],
  20: [[8, 12, 3, 10, 's'], [4, 16, 10, 11, 'h'], [5, 15, 11, 12, 'h'], [6, 14, 12, 13, 'h'], [7, 13, 13, 14, 'h'], [8, 12, 14, 15, 't'], [9, 11, 15, 16, 't']],
  24: [[9, 15, 5, 11, 's'], [5, 19, 11, 13, 'h'], [7, 17, 13, 15, 'h'], [9, 15, 15, 17, 't'], [11, 13, 17, 19, 't']],
  30: [[12, 18, 6, 14, 's'], [6, 24, 14, 16, 'h'], [8, 22, 16, 18, 'h'], [10, 20, 18, 20, 'h'], [12, 18, 20, 22, 't'], [14, 16, 22, 24, 't']],
  32: [[13, 19, 7, 15, 's'], [7, 25, 15, 17, 'h'], [9, 23, 17, 19, 'h'], [11, 21, 19, 21, 'h'], [13, 19, 21, 23, 't'], [15, 17, 23, 25, 't']],
  36: [[15, 21, 8, 18, 's'], [9, 27, 18, 20, 'h'], [11, 25, 20, 22, 'h'], [13, 23, 22, 24, 'h'], [15, 21, 24, 26, 't'], [17, 19, 26, 28, 't']],
  40: [[16, 24, 8, 19, 's'], [9, 31, 19, 21, 'h'], [11, 29, 21, 23, 'h'], [13, 27, 23, 25, 'h'], [15, 25, 25, 27, 'h'], [17, 23, 27, 29, 't'], [19, 21, 29, 31, 't']],
};
/**
 * The bare arrow (no tile) at 16 px, for the favicon on dark browser chrome:
 * 2 px chunks, 14 px wide.
 */
const BARE16 = [[5, 11, 1, 7, 's'], [1, 15, 7, 9, 'h'], [3, 13, 9, 11, 'h'], [5, 11, 11, 13, 't'], [7, 9, 13, 15, 't']];
/** The small tile's hairline: a 1 px white edge at 9% that keeps the tile visible on dark taskbars. */
const HAIRLINE = 0.09;

const bandRects = (rows, color = (b) => BAND[b]) =>
  rows.map(([x0, x1, y0, y1, b]) => `<rect x="${x0}" y="${y0}" width="${x1 - x0}" height="${y1 - y0}" fill="${color(b)}"/>`).join('');

function steppedSvg(size) {
  const rx = Math.round(0.2 * size);
  const body = [
    `  <rect width="${size}" height="${size}" rx="${rx}" fill="${INK}"/>`,
    `  <rect x="0.5" y="0.5" width="${size - 1}" height="${size - 1}" rx="${rx - 0.5}" fill="none" stroke="#fff" stroke-opacity="${HAIRLINE}"/>`,
    // crispEdges keeps the arrow on whole device pixels when a page places
    // the icon at a half-pixel offset.
    `  <g shape-rendering="crispEdges">${bandRects(STEPPED[size])}</g>`,
  ].join('\n');
  return svg({ w: size, h: size, title: `Boltget (${size} px)`, body });
}

/**
 * 48 to 128 px: the squircle tile with its gradient and rim, the arrow snapped
 * to whole pixels in flat row colours. 48: gapless 3 px cells; 60 to 80:
 * 1 px gaps; 96 and 128: pitch-snapped 1 px gaps with a 1 px corner.
 */
function midSvg(size) {
  const big = size >= 96;
  const pitch = Math.round((size * 0.56) / 9);
  const conf = size < 60 ? { px: 3, gap: 0 } : { px: pitch - 1, gap: 1 };
  const inset = big ? 5 : 2;
  const TW = size - 2 * inset;
  const tile = squircle(inset, inset, TW, TW, Math.round(0.22 * TW));
  const cells = snappedCells(size, conf.px, conf.gap);
  const defs = [
    tileGrad('tile'),
    `    <clipPath id="tc"><path d="${tile}"/></clipPath>`,
    vGrad('rimT', [[0, '#FFFFFF', 0.2], [1, '#FFFFFF', 0]], inset, inset + TW * 0.4),
    `    <radialGradient id="amb" gradientUnits="userSpaceOnUse" cx="${size / 2}" cy="${fmt(inset + TW * 0.58)}" r="${fmt(TW * 0.45)}">${stops([[0, SKY_LIGHT, 0.1], [1, SKY_LIGHT, 0]])}</radialGradient>`,
  ].join('\n');
  const body = [
    `  <path d="${tile}" fill="none" stroke="#000" stroke-opacity="0.25" stroke-width="1"/>`,
    `  <path d="${tile}" fill="url(#tile)"/>`,
    `  <g clip-path="url(#tc)"><rect width="${size}" height="${size}" fill="url(#amb)"/><path d="${tile}" fill="none" stroke="url(#rimT)" stroke-width="2"/></g>`,
    big ? '' : `  <path d="${tile}" fill="none" stroke="#fff" stroke-opacity="0.1" stroke-width="1" transform="translate(0.5 0.5) scale(${fmt((size - 1) / size)})"/>`,
    '  ' + cells.map(({ r, x, y, w, h }) => `<rect x="${x}" y="${y}" width="${w}" height="${h}" fill="${ROW_COLORS[r]}"${big ? ' rx="1"' : ''}/>`).join(''),
  ].filter(Boolean).join('\n');
  return svg({ w: size, h: size, title: `Boltget (${size} px)`, defs, body });
}

/** The native design for an icon of `size` px. */
function iconAt(size) {
  if (STEPPED[size]) return steppedSvg(size);
  if (size < 180) return midSvg(size);
  return masterSvg(size);
}
/** Sizes with a native design (written as svg/icon-N.svg). */
const NATIVE = [16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 128];
/** Windows' full icon set (ico/app.ico). */
const ICO_SIZES = [16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256];

function buildIcons() {
  writeSvg('icon.svg', masterSvg(1024));
  writeSvg('icon-animated.svg', masterSvg(1024, { animated: true, title: 'Boltget, animated' }));
  for (const s of NATIVE) writeSvg(`svg/icon-${s}.svg`, iconAt(s));
  writeSvg('svg/icon-square.svg', masterSvg(1024, { bleed: true, title: 'Boltget (square)' }));
  writeSvg('svg/icon-maskable.svg', masterSvg(1024, { bleed: true, title: 'Boltget (maskable)' }));
}

/* ------------------------------------------------------------------------ */
/* 8. The bare mark, wordmark and lockups                                    */
/* ------------------------------------------------------------------------ */

const MARK_VIEWBOX = `${GRID.origin} ${GRID.origin} ${GRID.n * U} ${GRID.n * U}`;
const markRamp = (id) => vGrad(id, RAMP, GRID.origin, GRID.origin + 9 * U);

function buildLogos() {
  const d = markPath();
  const markSvg = (title, fill, defs = '') =>
    svg({ w: 108, h: 108, viewBox: MARK_VIEWBOX, title, defs, body: `  <path fill="${fill}" d="${d}"/>` });

  writeSvg('svg/mark.svg', markSvg('Boltget mark (for dark backgrounds)', 'url(#g)', markRamp('g')));
  writeSvg('svg/mark-ink.svg', markSvg('Boltget mark, ink (for light backgrounds)', INK));
  writeSvg('svg/mark-white.svg', markSvg('Boltget mark, white', C.white));
  writeSvg('svg/mark-black.svg', markSvg('Boltget mark, black', C.black));

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
      defs: markRamp('g'),
      style: `.r{fill:url(#g);opacity:${MOTION.values[0]};animation:arrive ${MOTION.dur}s linear infinite}@keyframes arrive{${kf}}@media (prefers-reduced-motion:reduce){.r{animation:none;opacity:1}}`,
      body: rowsSvg,
    }),
  );

  // Favicon: the bare mark in one colour, like the site's big closing arrow:
  // ink on light browser chrome, white on dark. From 25 px up (a 2x tab gets
  // 32 device pixels) every cell is its own square; below that the gaps
  // would blur, so it switches to the 16 px stepped arrow, which fills the
  // tab like other sites' favicons.
  {
    const pitch = 3.25, px = 2.75, off = (32 - (8 * pitch + px)) / 2;
    const cells = CELLS.map(({ r, c }) => `<rect x="${off + c * pitch}" y="${off + r * pitch}" width="${px}" height="${px}"/>`).join('');
    const small = BARE16.map(([x0, x1, y0, y1]) => `<rect x="${x0 * 2}" y="${y0 * 2}" width="${(x1 - x0) * 2}" height="${(y1 - y0) * 2}"/>`).join('');
    writeSvg(
      'web/favicon.svg',
      svg({
        w: 32, h: 32, title: 'Boltget',
        style:
          `g{fill:${INK}}@media (prefers-color-scheme:dark){g{fill:#fff}}` +
          '.s{display:none}@media (max-width:24px){.b{display:none}.s{display:inline}}',
        body: [`  <g class="b">${cells}</g>`, `  <g class="s" shape-rendering="crispEdges">${small}</g>`].join('\n'),
      }),
    );
  }
  svgs['favicon-dark'] = svg({ w: 16, h: 16, title: 'x', body: bandRects(BARE16) });

  // Wordmark + lockups
  const WM_SIZE = 100;
  const wm = wordmarkText(WM_SIZE);
  const bb = wm.bbox;
  const wmBox = `${fmt(bb.x1)} ${fmt(bb.y1)} ${fmt(bb.x2 - bb.x1)} ${fmt(bb.y2 - bb.y1)}`;
  for (const [variant, fill] of [['on-dark', C.onDark], ['on-light', INK]]) {
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
  const variants = [['on-dark', 'url(#g)', markRamp('g'), C.onDark], ['on-light', INK, '', INK]];
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
    for (const [variant, fill, rowFill] of [['on-light', INK, () => INK], ['on-dark', C.onDark, (r) => ROW_COLORS[r]]]) {
      for (const [suffix, px] of [['', 2.5], ['@2x', 2.5]]) {
        const rects = CELLS.map(({ r, c }) => `<rect x="${c * pitch}" y="${r * pitch}" width="${px}" height="${px}" fill="${rowFill(r)}"/>`).join('');
        writeSvg(
          `svg/lockup-header-${variant}${suffix}.svg`,
          svg({ w: W, h: H, title: 'Boltget', body: `  ${rects}\n  <path fill="${fill}" d="${w.d}"/>` }),
        );
      }
    }
  }

  /*
   * Tile marks: the one-colour mark on a premium tile, for the website's
   * illustrations (the installer window on /download). Light: ink cells on a
   * white tile with a soft top-to-bottom sheen, a hairline and a short
   * shadow. Dark: white cells on the ink tile. Drawn on a 40 px grid with the
   * header's proportions (3 px pitch, 2.5 px cells), so they scale cleanly.
   */
  {
    const S = 40, pitch = 3, px = 2.5, off = (S - (8 * pitch + px)) / 2, rx = 9;
    const cells = (fill) => CELLS.map(({ r, c }) => `<rect x="${off + c * pitch}" y="${off + r * pitch}" width="${px}" height="${px}" rx="0.4" fill="${fill}"/>`).join('');
    const variants = [
      ['light', 'url(#t)', '#FFFFFF', '#EEF1F4', INK, 'rgb(19 33 43 / 0.10)'],
      ['dark', 'url(#t)', '#1B2D3A', '#0E1A23', '#FFFFFF', 'rgb(255 255 255 / 0.10)'],
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
  const line = NEUTRAL[700], faint = NEUTRAL[800], label = NEUTRAL[400], acc = RAMP[3][1];
  const mono = MONO('Regular');
  const L = (str, x, y, anchor = 'middle', fill = label, size = 13) => `  <path fill="${fill}" d="${text(str, { f: mono, size, x, y, anchor, decimals: 1 }).d}"/>`;
  const { inset, pitch, px, corner, radius } = MASTER;
  const O = (MASTER.size - (8 * pitch + px)) / 2;
  const parts = [];
  parts.push(`  <rect width="${W}" height="${H}" fill="${NEUTRAL[950]}"/>`);
  // artboard and tile
  parts.push(`  <rect x="${X(0)}" y="${Y(0)}" width="${fmt(MASTER.size * S)}" height="${fmt(MASTER.size * S)}" fill="none" stroke="${NEUTRAL[600]}" stroke-dasharray="4 6"/>`);
  parts.push(`  <path d="${squircle(X(inset), Y(inset), (MASTER.size - 2 * inset) * S, (MASTER.size - 2 * inset) * S, radius * S)}" fill="${INK}" stroke="${line}"/>`);
  // grid
  let gl = '';
  for (let i = 0; i <= 9; i++) {
    const p = O + i * pitch - (i === 9 ? pitch - px : 0);
    gl += `M${X(p)} ${Y(O)}V${Y(O + 8 * pitch + px)}M${X(O)} ${Y(p)}H${X(O + 8 * pitch + px)}`;
  }
  parts.push(`  <path fill="none" stroke="${faint}" d="${gl}"/>`);
  parts.push(`  <path fill="none" stroke="${line}" stroke-dasharray="2 4" d="M${X(512)} ${Y(0)}V${Y(1024)}M${X(0)} ${Y(512)}H${X(1024)}"/>`);
  // pixels
  parts.push(`  <g fill="url(#g)">${CELLS.map(({ r, c }) => `<rect x="${X(O + c * pitch)}" y="${Y(O + r * pitch)}" width="${fmt(px * S)}" height="${fmt(px * S)}" rx="${fmt(corner * S)}"/>`).join('')}</g>`);
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
  parts.push(L(`Artboard 1024 · tile ${1024 - 2 * inset} squircle · grid 9 × 9 · pitch ${pitch} · pixel ${px} (gap ${pitch - px}, corner ${corner}) · ramp ${RAMP.map((s) => s[1]).join(' → ')}`, 40, H - 34, 'start', NEUTRAL[500], 12));
  writeSvg(
    'svg/construction.svg',
    svg({ w: W, h: H, title: 'Boltget icon construction', defs: vGrad('g', RAMP, Y(O), Y(O + 8 * pitch + px)), body: parts.join('\n') }),
  );
}

/* ------------------------------------------------------------------------ */
/* 10. Social images                                                         */
/* ------------------------------------------------------------------------ */

/** The desktop app's 16 px line icons (apps/desktop/src/lib/Icon.svelte, as in apps/web/src/lib/appmarkup.ts). */
const APP_ICONS = {
  all: 'M2.5 4.5h11M2.5 8h11M2.5 11.5h11',
  active: 'M8 2.5v7.5M4.75 6.75 8 10l3.25-3.25M2.5 12v1.5h11V12',
  completed: 'M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11ZM5.5 8.25l1.75 1.75 3.25-3.5',
  scheduled: 'M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11ZM8 5v3.25l2.25 1.5',
  video: 'M2.5 3.5h11v9h-11ZM6.75 6v4l3.25-2Z',
  music: 'M6 11.5V3.5l7.5-1.5v8M6 11.5a1.75 1.75 0 1 1-3.5 0 1.75 1.75 0 0 1 3.5 0ZM13.5 10a1.75 1.75 0 1 1-3.5 0 1.75 1.75 0 0 1 3.5 0Z',
  archives: 'M2.5 3h11v3h-11ZM3.5 6v7h9V6M6.5 8.5h3',
  programs: 'M2.5 3.5h11v9h-11ZM2.5 6h11M4.5 4.75h1',
  documents: 'M3.5 2h5.5l3.5 3.5v8.5h-9ZM9 2v3.5h3.5M5.5 8.5h5M5.5 11h3.5',
  other: 'M2 4h4.5l1.5 1.5h6v7.5H2Z',
  queue: 'M2.5 4h7M2.5 8h7M2.5 12h7M12.5 3v10M11 11.5l1.5 1.5 1.5-1.5',
  search: 'M7 2.5a4.5 4.5 0 1 0 0 9 4.5 4.5 0 0 0 0-9ZM10.25 10.25 13.5 13.5',
  plus: 'M8 3v10M3 8h10',
};

/** The app's light theme (packages/ui tokens, as the site's HeroApp uses them). */
const APP = {
  chrome: '#F6F6F7', content: '#FFFFFF', hairline: [C.black, 0.08], hairlineStrong: [C.black, 0.14], border: NEUTRAL[300],
  stroke: '#84848D', text: C.black, muted: NEUTRAL[600], subtle: '#69696F', accent: EMERALD[700], tint: [EMERALD[600], 0.1],
  track: [C.black, 0.07], idle: '#84848D', progress: PROGRESS_LIGHT,
};

/**
 * The app window in its light theme, as the desktop app lays it out at its
 * minimum width (640 px, below 720: the sidebar is an icon rail and the
 * search field loses its key hint), drawn at `k` times its size with the
 * Home hero's demo downloads (apps/web/src/i18n/en.json, home.app).
 */
function appWindow(x0, y0, k) {
  const W = 640, H = 440;
  const X = (v) => fmt(x0 + v * k), Y = (v) => fmt(y0 + v * k), K = (v) => fmt(v * k);
  const fill = (c) => (Array.isArray(c) ? `fill="${c[0]}" fill-opacity="${c[1]}"` : `fill="${c}"`);
  const p = [];
  const label = (str, x, y, { size = 13, weight = 'Regular', color = APP.text, anchor = 'start' } = {}) => {
    const t = text(str, { f: SANS(weight), size: size * k, x: X(x), y: Y(y), anchor });
    p.push(`<path ${fill(color)} d="${t.d}"/>`);
    return t.width / k;
  };
  const glyph = (name, x, y, color, sw = 1.25) =>
    p.push(`<path d="${APP_ICONS[name]}" transform="translate(${X(x)} ${Y(y)}) scale(${k})" fill="none" stroke="${color}" stroke-width="${sw}" stroke-linecap="square" stroke-linejoin="miter"/>`);
  const rect = (x, y, w, h, color, r = 0) => p.push(`<rect x="${X(x)}" y="${Y(y)}" width="${K(w)}" height="${K(h)}"${r ? ` rx="${K(r)}"` : ''} ${fill(color)}/>`);

  // Window: chrome, its 1 px border and Windows 11's 8 px corners.
  p.push(`<rect x="${X(0)}" y="${Y(0)}" width="${K(W)}" height="${K(H)}" rx="${K(8)}" fill="${APP.chrome}"/>`);

  // Title bar (44): the rail's brand column (52) with the app icon, search, Add download, caption buttons.
  const iconPx = 16 * k;
  const iconSvg = iconAt(iconPx).replace(/<title>[^<]*<\/title>/, '');
  p.push(`<image x="${X(18)}" y="${Y(14)}" width="${iconPx}" height="${iconPx}" href="${dataUri(render(iconSvg, iconPx).asPng())}"/>`);
  const addText = text('Add download', { f: SANS('Medium'), size: 13 }).width;
  const btnW = 12 + 12 + 8 + addText + 12;
  const capsW = 3 * 46;
  const searchW = Math.min(380, W - 52 - capsW - (btnW + 24));
  rect(52, 8, searchW, 28, APP.hairlineStrong, 4);
  rect(53, 9, searchW - 2, 26, APP.content, 3);
  rect(53, 34, searchW - 2, 1, APP.stroke);
  glyph('search', 60, 14, APP.subtle);
  label('Search downloads', 84, 26, { color: APP.subtle });
  const bx = 52 + searchW + 12;
  rect(bx, 8, btnW, 28, APP.accent, 4);
  glyph('plus', bx + 8, 14, '#FFFFFF');
  label('Add download', bx + 32, 26, { weight: 'Medium', color: '#FFFFFF' });
  const cx = W - capsW;
  p.push(`<path d="M${X(cx + 18)} ${Y(22.5)}h${K(10)}M${X(46 + cx + 18.5)} ${Y(17.5)}h${K(9)}v${K(9)}h${K(-9)}ZM${X(92 + cx + 18.5)} ${Y(17.5)}l${K(9)} ${K(9)}m0 ${K(-9)}l${K(-9)} ${K(9)}" fill="none" stroke="${APP.text}" stroke-width="${k}"/>`);

  // Sidebar rail (52): views, a hairline, categories, a hairline, the queue.
  let y = 44 + 4;
  const item = (name, current = false) => {
    if (current) {
      rect(4, y, 44, 32, APP.tint, 4);
      rect(4, y + 8, 2, 16, APP.accent, 1);
    }
    glyph(name, 18, y + 8, current ? APP.accent : APP.muted);
    y += 33;
  };
  const rule = () => { y += 12; rect(12, y, 28, 1, APP.hairline); y += 13; };
  ['all', 'active', 'completed', 'scheduled'].forEach((n, i) => item(n, i === 0));
  rule();
  ['video', 'music', 'archives', 'programs', 'documents', 'other'].forEach((n) => item(n));
  rule();
  item('queue');

  // Content: white, with a hairline on its top and left and an 8 px corner.
  const L0 = 52, T0 = 44;
  p.push(`<path d="M${X(L0)} ${Y(H)}V${Y(T0 + 8)}Q${X(L0)} ${Y(T0)} ${X(L0 + 8)} ${Y(T0)}H${X(W)}V${Y(H)}Z" fill="${APP.content}"/>`);
  p.push(`<path d="M${X(L0 + 0.5)} ${Y(H)}V${Y(T0 + 8)}Q${X(L0 + 0.5)} ${Y(T0 + 0.5)} ${X(L0 + 8)} ${Y(T0 + 0.5)}H${X(W)}" fill="none" stroke="${APP.hairline[0]}" stroke-opacity="${APP.hairline[1]}" stroke-width="${k}"/>`);
  // List header (48): title and count.
  const tw = label('All downloads', L0 + 24, T0 + 29.5, { size: 15, weight: 'SemiBold' });
  label('11', L0 + 24 + tw + 8, T0 + 29.5, { color: APP.subtle });
  // Sections and rows (comfortable density, 40 px rows).
  y = T0 + 48;
  const rowX = L0 + 8, rowW = W - L0 - 16;
  const section = (name, count) => {
    const w = label(name, rowX + 16, y + 24, { size: 12, weight: 'Medium', color: APP.subtle });
    label(count, rowX + 16 + w + 8, y + 24, { size: 12, color: [APP.subtle, 0.8] });
    y += 36;
  };
  const barW = Math.max(72, Math.min(160, Math.round(0.18 * (rowW - 16 - 8 - 16 - 12 - 12 - 72))));
  const row = ({ name, kind, status, fraction, state, selected }) => {
    if (selected) {
      rect(rowX, y, rowW, 40, APP.tint, 4);
      rect(rowX, y + 8, 2, 24, APP.accent, 1);
    }
    glyph(kind, rowX + 16, y + 12, selected ? APP.muted : APP.subtle);
    const tx = rowX + 44;
    label(name, tx, y + 16);
    rect(tx, y + 27, barW, 4, APP.track);
    if (state === 'downloading') {
      p.push(`<rect x="${X(tx)}" y="${Y(y + 27)}" width="${K(barW * fraction)}" height="${K(4)}" fill="url(#progress)"/>`);
    } else {
      rect(tx, y + 27, barW * fraction, 4, APP.idle);
    }
    label(status, tx + barW + 8, y + 33.5, { size: 12, color: APP.subtle });
    y += 40;
  };
  section('Active', '3');
  row({ name: 'ubuntu-24.04.1-desktop-amd64.iso', kind: 'archives', status: '0.43 / 5.97 GiB · 8.70 MiB/s · about 11 min left', fraction: 0.072, state: 'downloading' });
  row({ name: 'Blender-4.2.3-windows-x64.msi', kind: 'programs', status: '319 / 342 MiB · 21.9 MiB/s · 2 s left', fraction: 0.933, state: 'downloading' });
  row({ name: 'big_buck_bunny_1080p_h264.mov', kind: 'video', status: '389 / 691 MiB · 9.72 MiB/s · 31 s left', fraction: 389 / 691, state: 'downloading', selected: true });
  section('Queued and paused', '1');
  row({ name: 'lecture-07-distributed-systems.pdf', kind: 'documents', status: '3.22 / 18 MiB · cs.example.edu', fraction: 0.18, state: 'paused' });

  const clip = `<clipPath id="win"><rect x="${X(0)}" y="${Y(0)}" width="${K(W)}" height="${K(H)}" rx="${K(8)}"/></clipPath>`;
  const defs = [
    clip,
    `<linearGradient id="progress" x1="0" y1="0" x2="1" y2="0">${stops(APP.progress)}</linearGradient>`,
    `<filter id="wsh" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="24"/></filter>`,
    `<filter id="wsh2" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="3"/></filter>`,
  ];
  const body = [
    `<rect x="${X(0)}" y="${Y(0) + 18}" width="${K(W)}" height="${K(H)}" rx="${K(8)}" fill="#0B2235" fill-opacity="0.16" filter="url(#wsh)"/>`,
    `<rect x="${X(0)}" y="${Y(0) + 2}" width="${K(W)}" height="${K(H)}" rx="${K(8)}" fill="#0B2235" fill-opacity="0.08" filter="url(#wsh2)"/>`,
    `<g clip-path="url(#win)">${p.join('')}</g>`,
    `<rect x="${X(0) + 0.5}" y="${Y(0) + 0.5}" width="${K(W) - 1}" height="${K(H) - 1}" rx="${K(8)}" fill="none" stroke="${APP.border}"/>`,
  ];
  return { defs, body: body.join('\n'), right: x0 + W * k, buttonRight: x0 + (bx + btnW) * k };
}

/** The sky photograph behind the social art, cropped to W x H (resvg cannot decode webp). */
async function skyPng(W, H) {
  return sharp(join(REPO, 'apps', 'web', 'src', 'assets', 'sky.webp')).resize(W, H, { fit: 'cover', position: 'top' }).png().toBuffer();
}

/**
 * The Open Graph card: sky, the icon and wordmark top-left, the headline
 * ("Every byte," in Geist, "at full speed." in Source Serif italic), the
 * serif subline, and the app window bleeding off the right and bottom. Every
 * text is outlined and at least 18 px.
 */
async function ogSvg(W, H, title) {
  const s = H / 630; // vertical scale against the 1200 x 630 card
  const LX = Math.round(80 * s), LY = Math.round(64 * s);
  const k = 1.5; // the window's scale: its 12 px text becomes 18
  const win = appWindow(Math.round(W - 750 * s), Math.round(170 * s), k);
  if (win.buttonRight > W - 12) throw new Error(`${title}: Add download falls off the card`);
  const P = [];
  P.push(`<image width="${W}" height="${H}" href="${dataUri(await skyPng(W, H))}"/>`);
  P.push(`<rect width="${W}" height="${H}" fill="url(#wash)"/>`);
  // logo
  const TS = Math.round(48 * s);
  P.push(`<image x="${LX}" y="${LY}" width="${TS}" height="${TS}" href="${dataUri(render(iconAt(TS), TS).asPng())}"/>`);
  const wmSize = 28 * s;
  const wm = wordmarkText(wmSize, LX + TS + Math.round(14 * s), LY + TS / 2 + capHeight(SANS(WORDMARK.weight), wmSize) / 2);
  P.push(`<path fill="${INK}" d="${wm.d}"/>`);
  // headline and subline
  const textRight = win.right - 640 * k - 36 * s;
  // The headline is as large as the column allows (at most 72 / 78 px).
  const widest = Math.max(
    text(COPY.headline[0], { f: SANS('SemiBold'), size: 72, tracking: -0.035 }).width,
    text(COPY.headline[1], { f: SERIF(true), size: 78, tracking: -0.012 }).width,
  );
  const fit = Math.min(s, (textRight - LX) / widest);
  const h1 = text(COPY.headline[0], { f: SANS('SemiBold'), size: 72 * fit, x: LX - 3 * fit, y: Math.round(280 * s), tracking: -0.035 });
  const h2 = text(COPY.headline[1], { f: SERIF(true), size: 78 * fit, x: LX - 1 * fit, y: Math.round(280 * s + 76 * fit), tracking: -0.012 });
  // The subline on one line if it fits at 22 px, else on two at 24.
  const subY = Math.round(280 * s + 76 * fit + 52 * s);
  const one = text(COPY.subline, { f: SERIF(false), size: 22 * s, x: LX, y: subY });
  const subs = LX + one.width <= textRight
    ? [one]
    : COPY.sublineLines.map((ln, i) => text(ln, { f: SERIF(false), size: 24 * s, x: LX, y: subY + i * 32 * s }));
  for (const [t, o] of [[h1, 1], [h2, 1], ...subs.map((t) => [t, 0.7])]) {
    if (t.x + t.width > textRight) throw new Error(`${title}: text runs into the window (${fmt(t.x + t.width)} > ${fmt(textRight)})`);
    P.push(`<path fill="${INK}"${o < 1 ? ` fill-opacity="${o}"` : ''} d="${t.d}"/>`);
  }
  P.push(win.body);
  const defs = [
    `<linearGradient id="wash" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="#fff" stop-opacity="0.28"/><stop offset="0.55" stop-color="#fff" stop-opacity="0"/></linearGradient>`,
    ...win.defs,
  ];
  return svg({ w: W, h: H, title, defs: defs.join('\n'), body: P.join('\n') });
}

/**
 * X header (1500 x 500): the sky, the headline on one line and the subline,
 * centred in the band X keeps clear of the avatar (lower left) and the
 * profile buttons (lower right).
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
  // GitHub social preview 1280 x 640 (the same card; the window may run into the 40 px border).
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
  for (const s of [...NATIVE, 256, 512, 1024]) rendered[s] = writePng(`png/icon-${s}.png`, iconAt(s), s, s);

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
  write(out('ico/app.ico'), encodeIco(ICO_SIZES.map((size) => ({ size, rendered: rendered[size] }))));
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
    neutral: NEUTRAL,
    emerald: EMERALD,
    semantic: SEMANTIC,
    theme: THEME,
    mark: { ink: INK, tile: TILE, ramp: RAMP, rows: ROW_COLORS },
    gradient: { progress: PROGRESS_DARK, progressLight: PROGRESS_LIGHT },
    atmosphere: ATMOSPHERE,
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
${v(NEUTRAL, 'neutral')}
${v(EMERALD, 'emerald')}
${v(ATMOSPHERE, 'atmosphere')}
  --mark-ink: ${INK};
${TILE.map((c, i) => `  --mark-tile-${i}: ${c};`).join('\n')}
  --gradient-mark: linear-gradient(180deg, ${RAMP.map(([o, c]) => `${c} ${fmt(o * 100)}%`).join(', ')});
  --gradient-tile: linear-gradient(120deg, ${TILE.join(', ')});
  --gradient-progress: linear-gradient(90deg, ${PROGRESS_DARK.map(([o, c]) => `${c} ${o * 100}%`).join(', ')});
  --gradient-progress-light: linear-gradient(90deg, ${PROGRESS_LIGHT.map(([o, c]) => `${c} ${o * 100}%`).join(', ')});
  --font-sans: "Geist", ui-sans-serif, system-ui, sans-serif;
  --font-mono: "Geist Mono", ui-monospace, "SFMono-Regular", Menlo, Consolas, monospace;
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
  const files = [
    ['geist-sans/Geist-Regular.woff2', 'Geist-Regular.woff2'],
    ['geist-sans/Geist-Medium.woff2', 'Geist-Medium.woff2'],
    ['geist-sans/Geist-SemiBold.woff2', 'Geist-SemiBold.woff2'],
    ['geist-mono/GeistMono-Regular.woff2', 'GeistMono-Regular.woff2'],
    ['geist-mono/GeistMono-Medium.woff2', 'GeistMono-Medium.woff2'],
  ];
  for (const [from, to] of files) {
    mkdirSync(out('fonts'), { recursive: true });
    copyFileSync(join(FONT_DIR, from), out('fonts', to));
    written.push(`fonts/${to}`);
  }
  copyFileSync(join(ROOT, 'node_modules', 'geist', 'LICENSE.TXT'), out('fonts', 'OFL.txt'));
  written.push('fonts/OFL.txt');
}

/* ------------------------------------------------------------------------ */
/* 13. preview.html                                                          */
/* ------------------------------------------------------------------------ */

function buildPreview() {
  const swatch = (name, c) => `<div class="sw"><span style="background:${c}"></span><b>${name}</b><code>${c}</code></div>`;
  const scale = (title, obj, prefix) =>
    `<h3>${title}</h3><div class="swatches">${Object.entries(obj).map(([k, c]) => swatch(`${prefix}-${k}`, c)).join('')}</div>`;
  const iconRow = [...NATIVE, 256].map((s) => `<figure><img src="png/icon-${s}.png" width="${s}" height="${s}" alt=""><figcaption>${s}</figcaption></figure>`).join('');
  const zoomRow = NATIVE.filter((s) => s <= 64).map((s) => `<figure><img class="px" src="png/icon-${s}.png" width="${s * (s > 32 ? 2 : 4)}" height="${s * (s > 32 ? 2 : 4)}" alt=""><figcaption>${s} @${s > 32 ? 2 : 4}×</figcaption></figure>`).join('');
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
@font-face{font-family:Geist;src:url(fonts/Geist-Regular.woff2) format("woff2");font-weight:400}
@font-face{font-family:Geist;src:url(fonts/Geist-Medium.woff2) format("woff2");font-weight:500}
@font-face{font-family:Geist;src:url(fonts/Geist-SemiBold.woff2) format("woff2");font-weight:600}
@font-face{font-family:"Geist Mono";src:url(fonts/GeistMono-Regular.woff2) format("woff2");font-weight:400}
:root{--bg:${NEUTRAL[950]};--surface:${NEUTRAL[900]};--border:${NEUTRAL[800]};--text:${NEUTRAL[50]};--muted:${NEUTRAL[400]};--subtle:${NEUTRAL[500]};--accent:${ROW_COLORS[6]}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--text);font:15px/1.55 Geist,system-ui,sans-serif;-webkit-font-smoothing:antialiased}
main{max-width:1180px;margin:0 auto;padding:56px 24px 96px}
header{display:flex;align-items:center;gap:20px;margin-bottom:56px}
header img{width:64px;height:64px}
h1{font-weight:600;font-size:28px;letter-spacing:-.02em;margin:0}
header p{margin:2px 0 0;color:var(--muted)}
h2{font-weight:600;font-size:13px;letter-spacing:.12em;text-transform:uppercase;color:var(--accent);font-family:"Geist Mono",monospace;margin:72px 0 20px;padding-top:24px;border-top:1px solid var(--border)}
h3{font-weight:500;font-size:15px;margin:28px 0 12px;color:var(--muted)}
h4{margin:0 0 12px;font-weight:500}
code{font-family:"Geist Mono",monospace;font-size:12px}
.duo{display:grid;grid-template-columns:1fr 1fr;border-radius:16px;overflow:hidden;border:1px solid var(--border)}
.duo>div{padding:32px;display:flex;flex-wrap:wrap;align-items:flex-end;justify-content:center;gap:24px;min-height:160px}
.on-dark{background:#1C1C1E}
.on-light{background:${NEUTRAL[0]};color:${NEUTRAL[950]}}
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
.theme.dark{background:${NEUTRAL[950]}}
.theme.light{background:${NEUTRAL[0]};color:${NEUTRAL[950]}}
@media (max-width:760px){.duo,.themes{grid-template-columns:1fr}}
</style>
</head>
<body>
<main>
<header><img src="icon-animated.svg" alt=""><div><h1>Boltget brand kit</h1><p>Generated by <code>scripts/build.mjs</code>. Every asset on dark and light.</p></div></header>

<h2>App icon</h2>
${both(() => `<figure><img src="icon.svg" width="256" height="256" alt=""><figcaption>icon.svg</figcaption></figure><figure><img src="icon-animated.svg" width="256" height="256" alt=""><figcaption>icon-animated.svg</figcaption></figure>`)}
<h3>Native sizes (16–40 stepped, 48–128 snapped, 256+ master)</h3>
${both(() => iconRow)}
<h3>Small sizes, magnified</h3>
${both(() => zoomRow)}
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
<div class="swatches">${swatch('ink', INK)}${TILE.map((c, i) => swatch(`tile-${i}`, c)).join('')}${RAMP.map(([o, c]) => swatch(`ramp ${o * 100}%`, c)).join('')}</div>
${scale('Emerald (app: progress, focus, success)', EMERALD, 'emerald')}
${scale('Neutral', NEUTRAL, 'neutral')}
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
