#!/usr/bin/env node
/**
 * BoltGet brand kit build.
 *
 * Regenerates every derived asset (SVG, PNG, ICO, tokens, preview.html) from the
 * single geometry + palette definition below. The two canonical files
 * (icon.svg, icon-animated.svg) are NOT rewritten; the build verifies that they
 * still match this geometry and fails if they drift.
 *
 *   pnpm install && pnpm build
 */
import { mkdirSync, readFileSync, writeFileSync, copyFileSync, rmSync } from 'node:fs';
import { dirname, join, resolve, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Resvg } from '@resvg/resvg-js';
import opentype from 'opentype.js';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
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
const ART = 180; // artboard
const TILE_RADIUS = 40; // squircle corner radius on the artboard
const GRID = { origin: 36, pitch: 12, inset: 1, px: 10, n: 9 };
const U = GRID.pitch; // the brand grid unit "u"

/** Animation signature (mirrors icon-animated.svg). */
const MOTION = {
  dur: 2.6, // s, full loop
  stagger: 0.12, // s, per row
  values: [0.15, 1, 1, 0.15],
  keyTimes: [0, 0.12, 0.85, 1],
};

const C = {
  ink: '#0B0B0F', // canvas / tile
  paper: '#F4F4F5', // light tile
  white: '#FFFFFF',
  green: '#34D399',
};

const NEUTRAL = {
  0: '#FFFFFF', 50: '#FAFAFA', 100: '#F4F4F5', 200: '#E4E4E7', 300: '#D4D4D8',
  400: '#A1A1AA', 500: '#71717A', 600: '#52525B', 700: '#3F3F46', 800: '#27272A',
  850: '#1C1C21', 900: '#141418', 950: '#0B0B0F',
};
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

/** Gradients: stops along the grid diagonal (36,36) -> (144,144). */
const GRAD_DARK = [[0, EMERALD[200]], [0.5, EMERALD[400]], [1, EMERALD[600]]];
const GRAD_LIGHT = [[0, EMERALD[400]], [0.5, EMERALD[500]], [1, EMERALD[700]]];

/** Copy. */
const COPY = {
  en: {
    label: 'DOWNLOAD MANAGER',
    tagline: ['Every byte, at full speed.'],
  },
  domain: 'boltget.app',
};

/* ------------------------------------------------------------------------ */
/* 2. Geometry helpers                                                       */
/* ------------------------------------------------------------------------ */

const fmt = (n) => +n.toFixed(3);

/** All lit cells as {r, c}. */
const CELLS = ROWS.flatMap(([a, b], r) => Array.from({ length: b - a + 1 }, (_, i) => ({ r, c: a + i })));

/** Path of the gapped mark (one subpath per pixel) for any grid. */
function markPath(g = GRID, cells = CELLS) {
  return cells
    .map(({ r, c }) => `M${fmt(g.origin + c * g.pitch + g.inset)} ${fmt(g.origin + r * g.pitch + g.inset)}h${fmt(g.px)}v${fmt(g.px)}h${fmt(-g.px)}z`)
    .join('');
}

/** Same as markPath but with an independent x/y origin. */
const gridAt = (x, y, pitch) => ({ ox: x, oy: y, pitch, inset: pitch / 12, px: (pitch * 10) / 12 });
function markPathAt(g, cells = CELLS) {
  return cells
    .map(({ r, c }) => `M${fmt(g.ox + c * g.pitch + g.inset)} ${fmt(g.oy + r * g.pitch + g.inset)}h${fmt(g.px)}v${fmt(g.px)}h${fmt(-g.px)}z`)
    .join('');
}

/**
 * Small-size (gapless) arrows, drawn directly in device pixels.
 * Each row: [x0, x1, y0, y1]. Rows are stacked top to bottom and traced into a
 * single stepped outline so the shape is one crisp, solid path.
 */
const SMALL = {
  // 16 px: 1 px steps, even width (10) so it centres on the pixel grid.
  16: { size: 16, radius: 3.5, box: [3, 13], rows: [[6, 10, 3, 8], [3, 13, 8, 9], [4, 12, 9, 10], [5, 11, 10, 11], [6, 10, 11, 12], [7, 9, 12, 13]] },
  // 24 px: 2 px "chunks" on a 7-cell arrow (14 px wide).
  24: { size: 24, radius: 5, box: [5, 19], rows: [[9, 15, 5, 11], [5, 19, 11, 13], [7, 17, 13, 15], [9, 15, 15, 17], [11, 13, 17, 19]] },
  // 16 px, no tile (dark-scheme favicon): 2 px chunks, 14 px wide.
  bare16: { size: 16, radius: 0, box: [1, 15], rows: [[5, 11, 1, 7], [1, 15, 7, 9], [3, 13, 9, 11], [5, 11, 11, 13], [7, 9, 13, 15]] },
};
/** Gapped mark snapped to whole pixels for sizes where 2/180 gaps would blur. */
const SNAPPED = { 64: { pitch: 4, gap: 1 }, 128: { pitch: 8, gap: 1 } };

function steppedPath(rows) {
  const right = [];
  const left = [];
  for (const [x0, x1, y0, y1] of rows) {
    right.push([x1, y0], [x1, y1]);
    left.push([x0, y0], [x0, y1]);
  }
  const pts = [...right, ...left.reverse()];
  // drop collinear points
  const clean = pts.filter((p, i) => {
    const a = pts[(i - 1 + pts.length) % pts.length];
    const b = pts[(i + 1) % pts.length];
    return !((a[0] === p[0] && p[0] === b[0]) || (a[1] === p[1] && p[1] === b[1]));
  });
  return 'M' + clean.map((p) => p.join(' ')).join('L') + 'Z';
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
function linGrad(id, stops, [x1, y1, x2, y2] = [36, 36, 144, 144]) {
  return [
    `    <linearGradient id="${id}" gradientUnits="userSpaceOnUse" x1="${fmt(x1)}" y1="${fmt(y1)}" x2="${fmt(x2)}" y2="${fmt(y2)}">`,
    ...stops.map(([o, c]) => `      <stop offset="${o}" stop-color="${c}"/>`),
    '    </linearGradient>',
  ].join('\n');
}
/** Gradient that spans the 9x9 grid box at any placement. */
const gridGrad = (id, stops, g) => linGrad(id, stops, [g.ox, g.oy, g.ox + 9 * g.pitch, g.oy + 9 * g.pitch]);
const tile = (fill, size = ART, r = TILE_RADIUS) => `  <rect width="${size}" height="${size}" rx="${r}" fill="${fill}"/>`;

/* ------------------------------------------------------------------------ */
/* 4. Type: Geist outlined with opentype.js                                  */
/* ------------------------------------------------------------------------ */

const fonts = {};
const font = (file) => (fonts[file] ??= opentype.loadSync(join(FONT_DIR, file)));
const SANS = (w) => font(`geist-sans/Geist-${w}.ttf`);
const MONO = (w) => font(`geist-mono/GeistMono-${w}.ttf`);

/** Optical kerning for the wordmark, in font units, added to the font's own GPOS kerning. */
const WORDMARK = { weight: 'SemiBold', tracking: -0.025, adjust: { Bo: -6, lt: 12, tG: 8 } };

/**
 * Lay out a single line as outlined path data.
 * anchor: 'start' | 'middle' | 'end'. Returns { d, width, bbox }.
 */
function text(str, { f, size, x = 0, y = 0, tracking = 0, adjust = {}, anchor = 'start', decimals = 2 }) {
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
const capHeight = (f, size) => (f.tables.os2.sCapHeight / f.unitsPerEm) * size;

function wordmarkText(size, x = 0, y = 0, anchor = 'start') {
  return text('BoltGet', { f: SANS(WORDMARK.weight), size, x, y, tracking: WORDMARK.tracking, adjust: WORDMARK.adjust, anchor });
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
function writeSvg(rel, text) {
  svgs[rel] = text;
  write(out(rel), text);
}
function writePng(rel, svgText, w, h) {
  const r = render(svgText, w, h);
  write(out(rel), r.asPng());
  return r;
}

/* ------------------------------------------------------------------------ */
/* 7. Canonical files: verify, never rewrite                                 */
/* ------------------------------------------------------------------------ */

function canonicalRects(r) {
  const [a, b] = ROWS[r];
  let s = '';
  for (let c = a; c <= b; c++) s += `<rect x="${GRID.origin + c * GRID.pitch + GRID.inset}" y="${GRID.origin + r * GRID.pitch + GRID.inset}" width="${GRID.px}" height="${GRID.px}"/>`;
  return s;
}
function canonicalIcon(animated) {
  const head = [
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${ART} ${ART}" width="${ART}" height="${ART}">`,
    '  <title>BoltGet</title>',
    '  <defs>',
    linGrad('g', GRAD_DARK),
    '  </defs>',
    tile(C.ink),
    '  <g fill="url(#g)">',
  ];
  const rows = ROWS.map((_, r) => {
    if (!animated) return `    ${canonicalRects(r)}`;
    const begin = +(r * MOTION.stagger).toFixed(2);
    return (
      `    <g opacity="${MOTION.values[0]}"><animate attributeName="opacity" values="${MOTION.values.join(';')}" keyTimes="${MOTION.keyTimes.join(';')}" dur="${MOTION.dur}s" begin="${begin}s" repeatCount="indefinite"/>\n` +
      `      ${canonicalRects(r)}</g>`
    );
  });
  return [...head, ...rows, '  </g>', '</svg>', ''].join('\n');
}
function verifyCanonical() {
  for (const [file, animated] of [['icon.svg', false], ['icon-animated.svg', true]]) {
    const actual = readFileSync(out(file), 'utf8').replace(/\r\n/g, '\n');
    if (actual !== canonicalIcon(animated)) {
      throw new Error(`${file} no longer matches the geometry in build.mjs. Update one or the other deliberately.`);
    }
  }
  svgs['icon.svg'] = canonicalIcon(false);
}

/* ------------------------------------------------------------------------ */
/* 8. Logo SVGs                                                              */
/* ------------------------------------------------------------------------ */

const MARK_VIEWBOX = `${GRID.origin} ${GRID.origin} ${GRID.n * U} ${GRID.n * U}`;

function buildLogos() {
  const d = markPath();
  const markSvg = (title, fill, defs = '') =>
    svg({ w: 108, h: 108, viewBox: MARK_VIEWBOX, title, defs, body: `  <path fill="${fill}" d="${d}"/>` });

  writeSvg('svg/mark.svg', markSvg('BoltGet mark', 'url(#g)', linGrad('g', GRAD_DARK)));
  writeSvg('svg/mark-light.svg', markSvg('BoltGet mark (for light backgrounds)', 'url(#g)', linGrad('g', GRAD_LIGHT)));
  writeSvg('svg/mark-white.svg', markSvg('BoltGet mark, white', C.white));
  writeSvg('svg/mark-black.svg', markSvg('BoltGet mark, black', C.ink));
  writeSvg('svg/mark-green.svg', markSvg('BoltGet mark, green', C.green));

  // Animated mark (transparent), CSS-driven so it honours prefers-reduced-motion.
  const kf = MOTION.keyTimes.map((t, i) => `${fmt(t * 100)}%{opacity:${MOTION.values[i]}}`).join('');
  const rowsSvg = ROWS.map((_, r) => {
    const cells = CELLS.filter((c) => c.r === r);
    return `  <path class="r" style="animation-delay:${fmt(r * MOTION.stagger)}s" d="${markPath(GRID, cells)}"/>`;
  }).join('\n');
  writeSvg(
    'svg/mark-animated.svg',
    svg({
      w: 108, h: 108, viewBox: MARK_VIEWBOX, title: 'BoltGet mark, animated',
      defs: linGrad('g', GRAD_DARK),
      style: `.r{fill:url(#g);opacity:${MOTION.values[0]};animation:arrive ${MOTION.dur}s linear infinite}@keyframes arrive{${kf}}@media (prefers-reduced-motion:reduce){.r{animation:none;opacity:1}}`,
      body: rowsSvg,
    }),
  );

  // App icons
  writeSvg(
    'svg/icon-light.svg',
    svg({ w: ART, h: ART, title: 'BoltGet (light)', defs: linGrad('g', GRAD_LIGHT), body: `${tile(C.paper)}\n  <path fill="url(#g)" d="${d}"/>` }),
  );
  const k = 0.9375; // keeps the arrow inside the Android 66/108 safe circle (and the 80% PWA zone)
  writeSvg(
    'svg/icon-maskable.svg',
    svg({
      w: ART, h: ART, title: 'BoltGet (maskable)', defs: linGrad('g', GRAD_DARK),
      body: `  <rect width="${ART}" height="${ART}" fill="${C.ink}"/>\n  <path fill="url(#g)" transform="matrix(${k} 0 0 ${k} ${fmt((ART / 2) * (1 - k))} ${fmt((ART / 2) * (1 - k))})" d="${d}"/>`,
    }),
  );
  // Full-bleed square at canonical proportions (apple-touch-icon, avatars).
  writeSvg(
    'svg/icon-square.svg',
    svg({ w: ART, h: ART, title: 'BoltGet (square)', defs: linGrad('g', GRAD_DARK), body: `  <rect width="${ART}" height="${ART}" fill="${C.ink}"/>\n  <path fill="url(#g)" d="${d}"/>` }),
  );

  // Small-size icons
  for (const key of ['16', '24']) {
    const s = SMALL[key];
    const [b0, b1] = s.box;
    writeSvg(
      `svg/icon-${key}.svg`,
      svg({
        w: s.size, h: s.size, title: `BoltGet (${key} px)`,
        defs: linGrad('g', GRAD_DARK, [b0, b0, b1, b1]),
        body: `${tile(C.ink, s.size, s.radius)}\n  <path fill="url(#g)" d="${steppedPath(s.rows)}"/>`,
      }),
    );
  }

  // Mid-size, pixel-snapped gapped icons: integer pitch and gap, centred on whole pixels.
  for (const [size, { pitch, gap }] of Object.entries(SNAPPED)) {
    const o = (size - 9 * pitch) / 2;
    const g = { ox: o, oy: o, pitch, inset: 0, px: pitch - gap };
    writeSvg(
      `svg/icon-${size}.svg`,
      svg({
        w: +size, h: +size, title: `BoltGet (${size} px)`,
        defs: gridGrad('g', GRAD_DARK, g),
        body: `${tile(C.ink, +size, Math.round((size * TILE_RADIUS) / ART))}
  <path fill="url(#g)" d="${markPathAt(g)}"/>`,
      }),
    );
  }

  // Favicon: dark tile on light browser chrome; bare, larger arrow on dark chrome.
  const s16 = SMALL[16], bare = SMALL.bare16;
  writeSvg(
    'web/favicon.svg',
    svg({
      w: 16, h: 16, title: 'BoltGet',
      style: '.d{display:none}@media (prefers-color-scheme:dark){.l{display:none}.d{display:inline}}',
      defs: [linGrad('a', GRAD_DARK, [s16.box[0], s16.box[0], s16.box[1], s16.box[1]]), linGrad('b', GRAD_DARK, [bare.box[0], bare.box[0], bare.box[1], bare.box[1]])].join('\n'),
      body: [
        `  <g class="l">\n  ${tile(C.ink, 16, s16.radius).trim()}\n    <path fill="url(#a)" d="${steppedPath(s16.rows)}"/>\n  </g>`,
        `  <path class="d" fill="url(#b)" d="${steppedPath(bare.rows)}"/>`,
      ].join('\n'),
    }),
  );
  // Same favicon with the scheme baked in, for rasterising/previewing each state.
  svgs['favicon-light'] = svg({ w: 16, h: 16, title: 'x', defs: linGrad('a', GRAD_DARK, [3, 3, 13, 13]), body: `${tile(C.ink, 16, s16.radius)}<path fill="url(#a)" d="${steppedPath(s16.rows)}"/>` });

  // Wordmark + lockups
  const WM_SIZE = 100;
  const wm = wordmarkText(WM_SIZE);
  const bb = wm.bbox;
  const wmBox = `${fmt(bb.x1)} ${fmt(bb.y1)} ${fmt(bb.x2 - bb.x1)} ${fmt(bb.y2 - bb.y1)}`;
  for (const [variant, fill] of [['on-dark', NEUTRAL[50]], ['on-light', C.ink]]) {
    writeSvg(
      `svg/wordmark-${variant}.svg`,
      svg({ w: fmt(bb.x2 - bb.x1), h: fmt(bb.y2 - bb.y1), viewBox: wmBox, title: 'BoltGet', body: `  <path fill="${fill}" d="${wm.d}"/>` }),
    );
  }

  /*
   * Lockup proportions, in grid units u (= pitch at artboard scale, 12):
   *   arrow ink  = 106 (9u - 2)        cap height = 4u (48)
   *   gap        = 2.5u between arrow ink and wordmark ink
   *   cap block centred on the arrow ink's vertical centre
   */
  const capU = 4;
  const wmSize = (capU * U) / (capHeight(SANS(WORDMARK.weight), 1));
  const ink = { x1: 37, y1: 37, x2: 143, y2: 143 };
  const midY = (ink.y1 + ink.y2) / 2;
  {
    const baseline = midY + (capU * U) / 2;
    const t = wordmarkText(wmSize, 0, 0);
    const x = ink.x2 + 2.5 * U - t.bbox.x1;
    const w2 = wordmarkText(wmSize, x, baseline);
    const vb = [ink.x1, ink.y1, w2.bbox.x2 - ink.x1, ink.y2 - ink.y1];
    for (const [variant, grad, fill] of [['on-dark', GRAD_DARK, NEUTRAL[50]], ['on-light', GRAD_LIGHT, C.ink]]) {
      writeSvg(
        `svg/lockup-horizontal-${variant}.svg`,
        svg({
          w: fmt(vb[2]), h: fmt(vb[3]), viewBox: vb.map(fmt).join(' '), title: 'BoltGet',
          defs: linGrad('g', grad),
          body: `  <path fill="url(#g)" d="${d}"/>\n  <path fill="${fill}" d="${w2.d}"/>`,
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
    // centre on ink rather than advance width
    const shift = ART / 2 - (w3.bbox.x1 + w3.bbox.x2) / 2;
    const w4 = wordmarkText(size, ART / 2 + shift, baseline, 'middle');
    const x1 = Math.min(ink.x1, w4.bbox.x1), x2 = Math.max(ink.x2, w4.bbox.x2);
    const vb = [x1, ink.y1, x2 - x1, baseline - ink.y1];
    for (const [variant, grad, fill] of [['on-dark', GRAD_DARK, NEUTRAL[50]], ['on-light', GRAD_LIGHT, C.ink]]) {
      writeSvg(
        `svg/lockup-stacked-${variant}.svg`,
        svg({
          w: fmt(vb[2]), h: fmt(vb[3]), viewBox: vb.map(fmt).join(' '), title: 'BoltGet',
          defs: linGrad('g', grad),
          body: `  <path fill="url(#g)" d="${d}"/>\n  <path fill="${fill}" d="${w4.d}"/>`,
        }),
      );
    }
  }
}

/* ------------------------------------------------------------------------ */
/* 9. Construction drawing                                                   */
/* ------------------------------------------------------------------------ */

function buildConstruction() {
  const S = 3; // drawing scale
  const W = 900, H = 860;
  const T = { x: 180, y: 150 }; // tile origin on canvas
  const X = (v) => T.x + v * S, Y = (v) => T.y + v * S;
  const line = NEUTRAL[700], faint = NEUTRAL[800], label = NEUTRAL[400], acc = EMERALD[400];
  const mono = MONO('Regular');
  const L = (str, x, y, anchor = 'middle', fill = label, size = 13) => `  <path fill="${fill}" d="${text(str, { f: mono, size, x, y, anchor, decimals: 1 }).d}"/>`;
  const parts = [];
  parts.push(`  <rect width="${W}" height="${H}" fill="${C.ink}"/>`);
  // clear space (2u around the tile)
  const cs = 2 * U * S;
  parts.push(`  <rect x="${X(0) - cs}" y="${Y(0) - cs}" width="${ART * S + 2 * cs}" height="${ART * S + 2 * cs}" fill="none" stroke="${NEUTRAL[600]}" stroke-dasharray="4 6"/>`);
  parts.push(L('clear space 2u', X(0) - cs, Y(0) - cs - 12, 'start'));
  // tile
  parts.push(`  <rect x="${X(0)}" y="${Y(0)}" width="${ART * S}" height="${ART * S}" rx="${TILE_RADIUS * S}" fill="${NEUTRAL[900]}" stroke="${line}"/>`);
  // grid
  let gl = '';
  for (let i = 0; i <= 9; i++) {
    const p = GRID.origin + i * U;
    gl += `M${X(p)} ${Y(36)}V${Y(144)}M${X(36)} ${Y(p)}H${X(144)}`;
  }
  parts.push(`  <path fill="none" stroke="${faint}" d="${gl}"/>`);
  // keylines: centre axes
  parts.push(`  <path fill="none" stroke="${line}" stroke-dasharray="2 4" d="M${X(90)} ${Y(0)}V${Y(180)}M${X(0)} ${Y(90)}H${X(180)}"/>`);
  // pixels
  const g = { ox: X(36), oy: Y(36), pitch: U * S, inset: S, px: 10 * S };
  parts.push(`  <path fill="url(#g)" d="${markPathAt(g)}"/>`);
  // row / column indices
  for (let i = 0; i < 9; i++) {
    parts.push(L(String(i), X(36 + i * U + 6), Y(36) - 10));
    parts.push(L(String(i), X(36) - 12, Y(36 + i * U + 6) + 4, 'end'));
  }
  // origin marker
  parts.push(`  <circle cx="${X(36)}" cy="${Y(36)}" r="4" fill="${acc}"/>`);
  parts.push(L('origin (36, 36)', X(36) - 12, Y(36) - 28, 'end', acc));

  // dimension helpers
  const dimH = (x1, x2, y, str, tick = 6) =>
    `  <path fill="none" stroke="${label}" d="M${x1} ${y}H${x2}M${x1} ${y - tick}V${y + tick}M${x2} ${y - tick}V${y + tick}"/>\n` + L(str, (x1 + x2) / 2, y - 10);
  const dimV = (y1, y2, x, str, tick = 6, side = 'end') =>
    `  <path fill="none" stroke="${label}" d="M${x} ${y1}V${y2}M${x - tick} ${y1}H${x + tick}M${x - tick} ${y2}H${x + tick}"/>\n` +
    L(str, side === 'end' ? x - 12 : x + 12, (y1 + y2) / 2 + 4, side);

  // top: artboard
  parts.push(dimH(X(0), X(180), Y(0) - cs - 44, '180'));
  // bottom: margin + grid
  const yb = Y(180) + cs + 40;
  parts.push(dimH(X(0), X(36), yb, '36'));
  parts.push(dimH(X(36), X(144), yb, '108 = 9u'));
  parts.push(dimH(X(144), X(180), yb, '36'));
  // right: pixel detail on rows 0..1 of the shaft
  const xr = X(180) + cs + 40;
  parts.push(dimV(Y(36), Y(48), xr, 'u = 12', 6, 'start'));
  parts.push(dimV(Y(37), Y(47), xr - 40, '10', 6, 'end'));
  parts.push(`  <path fill="none" stroke="${NEUTRAL[600]}" stroke-dasharray="2 3" d="M${X(107)} ${Y(37)}H${xr - 40}M${X(107)} ${Y(47)}H${xr - 40}M${X(107)} ${Y(48)}H${xr}M${X(107)} ${Y(36)}H${xr}"/>`);
  // gap callout between col 3 and col 4 on row 2
  parts.push(`  <path fill="none" stroke="${acc}" d="M${xr - 40} ${Y(47)}V${Y(49)}M${xr - 46} ${Y(49)}H${xr - 34}"/>`);
  parts.push(`  <path fill="none" stroke="${acc}" d="M${xr - 40} ${Y(49)}V${Y(49) + 20}H${xr - 50}"/>`);
  parts.push(L('gap 2', xr - 56, Y(49) + 24, 'end', acc));
  parts.push(`  <path fill="none" stroke="${NEUTRAL[600]}" stroke-dasharray="2 3" d="M${X(107)} ${Y(49)}H${xr - 40}"/>`);
  // radius callout
  parts.push(`  <path fill="none" stroke="${acc}" d="M${X(0)} ${Y(40)}A${40 * S} ${40 * S} 0 0 1 ${X(40)} ${Y(0)}"/>`);
  parts.push(L('r = 40', X(14), Y(14) + 4, 'middle', acc));
  // legend
  const lx = 40, ly = H - 34;
  parts.push(L('Artboard 180 · grid 9 × 9 · pitch u = 12 · pixel 10 × 10 at +1 · square corners · gradient (36,36) → (144,144)', lx, ly, 'start', NEUTRAL[500], 12));

  writeSvg(
    'svg/construction.svg',
    svg({
      w: W, h: H, title: 'BoltGet mark construction',
      defs: linGrad('g', GRAD_DARK, [X(36), Y(36), X(144), Y(144)]),
      body: parts.join('\n'),
    }),
  );
}

/* ------------------------------------------------------------------------ */
/* 10. Social images                                                         */
/* ------------------------------------------------------------------------ */

/**
 * A "field of chunks": the 9x9 grid pattern extended across the canvas at a
 * whisper, so the lit arrow reads as the chunks that have arrived.
 */
function socialSvg({ W, H, pitch, markX, markY, textX, textY, scale = 1, title, align = 'start' }) {
  const g = gridAt(markX, markY, pitch);
  const cx = markX + 4.5 * pitch, cy = markY + 4.5 * pitch;
  const copy = COPY.en;
  const sans = SANS('Medium');
  const mono = MONO('Regular');

  const labelSize = 15 * scale, wmSize = 92 * scale, tagSize = 34 * scale, domainSize = 18 * scale;
  const capWm = capHeight(SANS(WORDMARK.weight), wmSize);
  const lines = [];
  let y = textY; // top of the text block
  const label = text(copy.label, { f: mono, size: labelSize, x: textX, y: y + capHeight(mono, labelSize), tracking: 0.14, anchor: align });
  lines.push(`  <path fill="${EMERALD[400]}" d="${label.d}"/>`);
  y += capHeight(mono, labelSize) + 30 * scale + capWm;
  const wm = wordmarkText(wmSize, textX, y, align);
  lines.push(`  <path fill="${NEUTRAL[50]}" d="${wm.d}"/>`);
  y += 30 * scale;
  let maxRight = Math.max(label.bbox.x2, wm.bbox.x2);
  copy.tagline.forEach((ln, i) => {
    y += tagSize * (i === 0 ? 1.05 : 1.3);
    const t = text(ln, { f: sans, size: tagSize, x: textX, y, tracking: -0.01, anchor: align });
    maxRight = Math.max(maxRight, t.bbox.x2);
    lines.push(`  <path fill="${NEUTRAL[400]}" d="${t.d}"/>`);
  });
  const textBottom = y;

  // trail: three ghost rows above the shaft (the chunks still arriving)
  const trail = [0.28, 0.13, 0.06].map((o, i) => {
    const cells = [3, 4, 5].map((c) => ({ r: -(i + 1), c }));
    return `  <path fill="${EMERALD[200]}" opacity="${o}" d="${markPathAt(g, cells)}"/>`;
  });

  const body = [
    `  <rect width="${W}" height="${H}" fill="${C.ink}"/>`,
    `  <circle cx="${fmt(cx)}" cy="${fmt(cy)}" r="${fmt(pitch * 11)}" fill="url(#amb)"/>`,
    `  <rect width="${W}" height="${H}" fill="url(#cells)" mask="url(#fade)"/>`,
    ...trail,
    `  <path fill="url(#g)" d="${markPathAt(g)}"/>`,
    ...lines,
    `  <path fill="${NEUTRAL[500]}" d="${text(COPY.domain, { f: mono, size: domainSize, x: textX, y: H - 64 * scale, anchor: align }).d}"/>`,
  ].join('\n');

  const defs = [
    gridGrad('g', GRAD_DARK, g),
    `    <radialGradient id="amb" cx="0.5" cy="0.5" r="0.5">\n      <stop offset="0" stop-color="${EMERALD[500]}" stop-opacity="0.13"/>\n      <stop offset="1" stop-color="${EMERALD[500]}" stop-opacity="0"/>\n    </radialGradient>`,
    `    <radialGradient id="fadeg" gradientUnits="userSpaceOnUse" cx="${fmt(cx)}" cy="${fmt(cy)}" r="${fmt(pitch * 14)}">\n      <stop offset="0" stop-color="#fff" stop-opacity="1"/>\n      <stop offset="1" stop-color="#fff" stop-opacity="0"/>\n    </radialGradient>`,
    `    <mask id="fade" maskUnits="userSpaceOnUse" x="0" y="0" width="${W}" height="${H}">\n      <rect width="${W}" height="${H}" fill="url(#fadeg)"/>\n    </mask>`,
    `    <pattern id="cells" patternUnits="userSpaceOnUse" x="${fmt(markX)}" y="${fmt(markY)}" width="${fmt(pitch)}" height="${fmt(pitch)}">\n      <rect x="${fmt(g.inset)}" y="${fmt(g.inset)}" width="${fmt(g.px)}" height="${fmt(g.px)}" fill="#fff" fill-opacity="0.045"/>\n    </pattern>`,
  ].join('\n');

  // layout guard: text must not collide with the mark or leave the canvas
  const markLeft = markX;
  if (align === 'start' && maxRight > markLeft - 40) throw new Error(`${title}: text (${maxRight}) runs into the mark (${markLeft})`);
  if (textBottom > H - 64 * scale - domainSize * 2) throw new Error(`${title}: text block too tall`);

  return svg({ w: W, h: H, title, defs, body });
}

function buildSocial() {
  // Open Graph 1200x630 (pitch 36 -> 324 px arrow)
  const og = socialSvg({ W: 1200, H: 630, pitch: 36, markX: 762, markY: 171, textX: 88, textY: 196, title: 'BoltGet — every byte, at full speed' });
  writeSvg('social/og-image.svg', og);
  writePng('social/og-image.png', og, 1200, 630);

  // GitHub social preview 1280x640 (keep 40 px safe border)
  const gh = socialSvg({ W: 1280, H: 640, pitch: 38, markX: 808, markY: 167, textX: 96, textY: 194, scale: 1.04, title: 'BoltGet — every byte, at full speed' });
  writeSvg('social/github-social.svg', gh);
  writePng('social/github-social.png', gh, 1280, 640);

  // X / Twitter header 1500x500: content kept to the centre band and away from
  // the avatar, which overlaps the lower-left corner.
  const x = socialSvg({ W: 1500, H: 500, pitch: 28, markX: 1112, markY: 124, textX: 470, textY: 136, scale: 0.92, title: 'BoltGet — every byte, at full speed' });
  writeSvg('social/x-header.svg', x);
  writePng('social/x-header.png', x, 1500, 500);

  // Avatar: full bleed (platforms crop to a circle), arrow at canonical proportions.
  const avatar = svgs['svg/icon-square.svg'].replace('BoltGet (square)', 'BoltGet avatar');
  writeSvg('social/avatar.svg', avatar);
  writePng('social/avatar-400.png', avatar, 400, 400);
  writePng('social/avatar-512.png', avatar, 512, 512);
}

/* ------------------------------------------------------------------------ */
/* 11. Rasters + ICO                                                         */
/* ------------------------------------------------------------------------ */

/** Which source draws which pixel size (small sizes use the gapless designs). */
function iconSourceFor(size) {
  if (size === 16 || size === 32) return svgs['svg/icon-16.svg'];
  if (size === 24 || size === 48) return svgs['svg/icon-24.svg'];
  if (SNAPPED[size]) return svgs[`svg/icon-${size}.svg`];
  return svgs['icon.svg'];
}

function buildRasters() {
  const sizes = [16, 24, 32, 48, 64, 128, 256, 512, 1024];
  const rendered = {};
  for (const s of sizes) rendered[s] = writePng(`png/icon-${s}.png`, iconSourceFor(s), s, s);
  for (const s of [256, 512, 1024]) writePng(`png/icon-light-${s}.png`, svgs['svg/icon-light.svg'], s, s);

  // Lockups as PNG (for slides/docs) at a fixed height of 240 px.
  for (const n of ['lockup-horizontal-on-dark', 'lockup-horizontal-on-light', 'lockup-stacked-on-dark', 'lockup-stacked-on-light']) {
    const src = svgs[`svg/${n}.svg`];
    const [, , vw, vh] = src.match(/viewBox="([^"]+)"/)[1].split(' ').map(Number);
    const h = 240;
    writePng(`png/${n}.png`, src, Math.round((vw / vh) * h));
  }

  // Web
  writePng('web/apple-touch-icon.png', svgs['svg/icon-square.svg'], 180, 180);
  writePng('web/icon-192.png', svgs['icon.svg'], 192, 192);
  writePng('web/icon-512.png', svgs['icon.svg'], 512, 512);
  writePng('web/icon-maskable-512.png', svgs['svg/icon-maskable.svg'], 512, 512);
  write(out('web/favicon.ico'), encodeIco([16, 32, 48].map((size) => ({ size, rendered: rendered[size] }))));
  write(
    out('web/site.webmanifest'),
    JSON.stringify(
      {
        name: 'BoltGet',
        short_name: 'BoltGet',
        icons: [
          { src: '/icon-192.png', sizes: '192x192', type: 'image/png' },
          { src: '/icon-512.png', sizes: '512x512', type: 'image/png' },
          { src: '/icon-maskable-512.png', sizes: '512x512', type: 'image/png', purpose: 'maskable' },
        ],
        theme_color: C.ink,
        background_color: C.ink,
        display: 'standalone',
      },
      null,
      2,
    ) + '\n',
  );

  // Windows app icon
  write(out('ico/app.ico'), encodeIco([16, 24, 32, 48, 64, 128, 256].map((size) => ({ size, rendered: rendered[size] }))));
}

/* ------------------------------------------------------------------------ */
/* 12. Tokens + fonts                                                        */
/* ------------------------------------------------------------------------ */

function buildTokens() {
  const tokens = { neutral: NEUTRAL, emerald: EMERALD, semantic: SEMANTIC, theme: THEME, gradient: { mark: GRAD_DARK, markLight: GRAD_LIGHT }, motion: MOTION };
  write(out('tokens/tokens.json'), JSON.stringify(tokens, null, 2) + '\n');
  const v = (o, p) => Object.entries(o).map(([k, c]) => `  --${p}-${k}: ${c};`).join('\n');
  const theme = (t) => [
    ...Object.entries(THEME[t]).map(([k, c]) => `  --bg-${k}: ${c};`.replace('--bg-bg', '--bg')),
    ...Object.entries(SEMANTIC).map(([k, c]) => `  --${k}: ${c[t]};`),
  ].join('\n');
  const css = `/* BoltGet design tokens — generated by scripts/build.mjs */
:root {
${v(NEUTRAL, 'neutral')}
${v(EMERALD, 'emerald')}
  --gradient-mark: linear-gradient(135deg, ${GRAD_DARK.map(([o, c]) => `${c} ${o * 100}%`).join(', ')});
  --gradient-mark-light: linear-gradient(135deg, ${GRAD_LIGHT.map(([o, c]) => `${c} ${o * 100}%`).join(', ')});
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
  const swatch = (name, hex, dark) => `<div class="sw"><span style="background:${hex}"></span><b>${name}</b><code>${hex}</code></div>`;
  const scale = (title, obj, prefix) =>
    `<h3>${title}</h3><div class="swatches">${Object.entries(obj).map(([k, c]) => swatch(`${prefix}-${k}`, c)).join('')}</div>`;
  const sizes = [16, 24, 32, 48, 64, 128, 256];
  const iconRow = sizes.map((s) => `<figure><img src="png/icon-${s}.png" width="${s}" height="${s}" alt=""><figcaption>${s}</figcaption></figure>`).join('');
  const iconRow2x = [16, 24, 32, 48, 64].map((s) => `<figure><img class="px" src="png/icon-${s}.png" width="${s * (s > 32 ? 3 : 4)}" height="${s * (s > 32 ? 3 : 4)}" alt=""><figcaption>${s} @${s > 32 ? 3 : 4}×</figcaption></figure>`).join('');
  const both = (inner, cls = '') => `<div class="duo ${cls}"><div class="on-dark">${inner('dark')}</div><div class="on-light">${inner('light')}</div></div>`;
  const theme = (t) =>
    `<div class="theme ${t}"><h4>${t === 'dark' ? 'Dark theme' : 'Light theme'}</h4>${Object.entries(THEME[t]).map(([k, c]) => swatch(k, c)).join('')}${Object.entries(SEMANTIC).map(([k, c]) => swatch(k, c[t])).join('')}</div>`;

  const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>BoltGet brand kit</title>
<link rel="icon" href="web/favicon.svg" type="image/svg+xml">
<style>
@font-face{font-family:Geist;src:url(fonts/Geist-Regular.woff2) format("woff2");font-weight:400}
@font-face{font-family:Geist;src:url(fonts/Geist-Medium.woff2) format("woff2");font-weight:500}
@font-face{font-family:Geist;src:url(fonts/Geist-SemiBold.woff2) format("woff2");font-weight:600}
@font-face{font-family:"Geist Mono";src:url(fonts/GeistMono-Regular.woff2) format("woff2");font-weight:400}
@font-face{font-family:"Geist Mono";src:url(fonts/GeistMono-Medium.woff2) format("woff2");font-weight:500}
:root{--bg:${NEUTRAL[950]};--surface:${NEUTRAL[900]};--border:${NEUTRAL[800]};--text:${NEUTRAL[50]};--muted:${NEUTRAL[400]};--subtle:${NEUTRAL[500]};--accent:${EMERALD[400]}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--text);font:15px/1.55 Geist,system-ui,sans-serif;-webkit-font-smoothing:antialiased}
main{max-width:1180px;margin:0 auto;padding:56px 24px 96px}
header{display:flex;align-items:center;gap:20px;margin-bottom:56px}
header img{width:56px;height:56px}
h1{font-weight:600;font-size:28px;letter-spacing:-.02em;margin:0}
header p{margin:2px 0 0;color:var(--muted)}
h2{font-weight:600;font-size:13px;letter-spacing:.12em;text-transform:uppercase;color:var(--accent);font-family:"Geist Mono",monospace;margin:72px 0 20px;padding-top:24px;border-top:1px solid var(--border)}
h3{font-weight:500;font-size:15px;margin:28px 0 12px;color:var(--muted)}
h4{margin:0 0 12px;font-weight:500}
code,.mono{font-family:"Geist Mono",monospace;font-size:12px}
.duo{display:grid;grid-template-columns:1fr 1fr;border-radius:16px;overflow:hidden;border:1px solid var(--border)}
.duo>div{padding:32px;display:flex;flex-wrap:wrap;align-items:flex-end;justify-content:center;gap:28px;min-height:160px}
.on-dark{background:${NEUTRAL[850]}}
.on-light{background:${NEUTRAL[0]};color:${NEUTRAL[950]}}
figure{margin:0;display:flex;flex-direction:column;align-items:center;gap:8px}
figcaption{font:12px "Geist Mono",monospace;color:var(--subtle)}
.px{image-rendering:pixelated}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(250px,1fr));gap:16px}
.card{background:var(--surface);border:1px solid var(--border);border-radius:14px;padding:20px;display:flex;flex-direction:column;gap:12px;align-items:center}
.card.light{background:${NEUTRAL[100]}}
.card img{max-width:100%}
.social img{width:100%;border-radius:10px;border:1px solid var(--border);display:block}
.social figure{align-items:stretch}
.social{display:grid;grid-template-columns:1fr 1fr;gap:24px}
.swatches{display:grid;grid-template-columns:repeat(auto-fill,minmax(150px,1fr));gap:10px}
.sw{display:grid;grid-template-columns:36px 1fr;grid-template-rows:auto auto;column-gap:10px;align-items:center;font-size:13px}
.sw span{grid-row:span 2;width:36px;height:36px;border-radius:8px;border:1px solid #ffffff1a}
.sw b{font-weight:500}.sw code{color:var(--subtle)}
.themes{display:grid;grid-template-columns:1fr 1fr;gap:16px}
.theme{border-radius:14px;padding:20px;display:grid;grid-template-columns:repeat(auto-fill,minmax(150px,1fr));gap:10px;border:1px solid var(--border)}
.theme h4{grid-column:1/-1}
.theme.dark{background:${NEUTRAL[950]}}
.theme.light{background:${NEUTRAL[0]};color:${NEUTRAL[950]}}
.theme.light .sw span{border-color:#0000001a}
.type .row{display:grid;grid-template-columns:140px 1fr;gap:16px;align-items:baseline;padding:10px 0;border-bottom:1px solid var(--border)}
.type .row code{color:var(--subtle)}
.anim{width:180px;height:180px}
@media (max-width:760px){.duo,.social,.themes{grid-template-columns:1fr}.type .row{grid-template-columns:1fr}}
</style>
</head>
<body>
<main>
<header><img src="icon-animated.svg" alt=""><div><h1>BoltGet brand kit</h1><p>Generated by <code>scripts/build.mjs</code>. Every asset on dark and light.</p></div></header>

<h2>Mark</h2>
${both((t) => `<figure><img src="${t === 'dark' ? 'icon.svg' : 'svg/icon-light.svg'}" width="180" height="180" alt=""><figcaption>${t === 'dark' ? 'icon.svg' : 'svg/icon-light.svg'}</figcaption></figure><figure><img class="anim" src="icon-animated.svg" alt=""><figcaption>icon-animated.svg</figcaption></figure>`)}
<h3>Mark only</h3>
${both((t) => (t === 'dark' ? ['mark', 'mark-animated', 'mark-white', 'mark-green'] : ['mark-light', 'mark-black', 'mark-green']).map((n) => `<figure><img src="svg/${n}.svg" width="96" height="96" alt=""><figcaption>${n}</figcaption></figure>`).join(''))}

<h2>App icon sizes</h2>
${both(() => iconRow)}
<h3>Pixel-snapped designs, magnified (16/32 ← icon-16.svg, 24/48 ← icon-24.svg, 64 ← icon-64.svg)</h3>
${both(() => iconRow2x)}
<h3>Favicon (light / dark browser chrome), web and maskable</h3>
${both((t) => `<figure><img class="px" src="${t === 'dark' ? 'web/favicon.svg' : 'png/icon-16.png'}" width="64" height="64" alt=""><figcaption>favicon ${t === 'dark' ? '(scheme-aware)' : '16'}</figcaption></figure><figure><img src="web/apple-touch-icon.png" width="90" height="90" alt=""><figcaption>apple-touch-icon</figcaption></figure><figure><img src="web/icon-maskable-512.png" width="90" height="90" style="border-radius:50%" alt=""><figcaption>maskable (circle crop)</figcaption></figure>`)}

<h2>Wordmark &amp; lockups</h2>
${both((t) => `<figure><img src="svg/wordmark-on-${t}.svg" height="44" alt=""><figcaption>wordmark-on-${t}</figcaption></figure>`)}
<div style="height:16px"></div>
${both((t) => `<figure><img src="svg/lockup-horizontal-on-${t}.svg" height="72" alt=""><figcaption>lockup-horizontal-on-${t}</figcaption></figure>`)}
<div style="height:16px"></div>
${both((t) => `<figure><img src="svg/lockup-stacked-on-${t}.svg" height="160" alt=""><figcaption>lockup-stacked-on-${t}</figcaption></figure>`)}
<h3>Small lockup check</h3>
${both((t) => `<img src="svg/lockup-horizontal-on-${t}.svg" height="24" alt=""><img src="svg/lockup-horizontal-on-${t}.svg" height="32" alt="">`)}

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
<figure><img src="social/avatar-400.png" width="160" height="160" style="border-radius:24px" alt=""><figcaption>avatar · rounded crop</figcaption></figure>
<figure><img src="social/avatar-400.png" width="40" height="40" style="border-radius:50%" alt=""><figcaption>40</figcaption></figure>
</div>

<h2>Color</h2>
${scale('Emerald', EMERALD, 'emerald')}
${scale('Neutral', NEUTRAL, 'neutral')}
<h3>Mark gradient</h3>
<div class="swatches" style="grid-template-columns:1fr 1fr">
<div style="height:56px;border-radius:10px;background:linear-gradient(90deg,${GRAD_DARK.map(([o, c]) => `${c} ${o * 100}%`).join(',')})"></div>
<div style="height:56px;border-radius:10px;background:linear-gradient(90deg,${GRAD_LIGHT.map(([o, c]) => `${c} ${o * 100}%`).join(',')})"></div>
<code>dark: ${GRAD_DARK.map((s) => s[1]).join(' → ')}</code><code>light: ${GRAD_LIGHT.map((s) => s[1]).join(' → ')}</code>
</div>
<h3>Themes &amp; semantic colors</h3>
<div class="themes">${theme('dark')}${theme('light')}</div>

<h2>Typography</h2>
<div class="type">
<div class="row"><code>Display 56/600</code><div style="font-size:56px;font-weight:600;letter-spacing:-.035em;line-height:1.05">Every byte, at full speed.</div></div>
<div class="row"><code>H1 40/600</code><div style="font-size:40px;font-weight:600;letter-spacing:-.03em;line-height:1.1">Picks up exactly where it stopped.</div></div>
<div class="row"><code>H2 28/600</code><div style="font-size:28px;font-weight:600;letter-spacing:-.02em">Every connection stays busy.</div></div>
<div class="row"><code>Body 15/400</code><div>BoltGet splits each file into chunks and fetches them in parallel, then assembles them on disk.</div></div>
<div class="row"><code>Mono 13/400</code><div class="mono" style="font-size:13px">4.2 GB · 16 connections · 98.4 MB/s · 00:43 left</div></div>
</div>
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
verifyCanonical();
buildLogos();
buildConstruction();
buildSocial();
buildRasters();
buildTokens();
copyFonts();
buildPreview();
console.log(`BoltGet brand: wrote ${written.length} files`);
for (const f of written.sort()) console.log('  ' + f);
