// The README's artwork: link buttons, the platform row and the stack pills,
// each in a dark and a light variant (the README picks one with <picture>).
// Text is outlined from Red Hat Text / Display, so GitHub, which shows SVGs
// as images without web fonts, draws them exactly. Monochrome like the rest
// of the brand, with a soft vertical gradient on fills and type.
//
//   pnpm --filter @boltget/brand readme
//
// Writes .github/assets/readme/*.svg. Edit this script, not the outputs.

import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { create as openFont } from 'fontkitten';

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, '..', '..');
const OUT = join(REPO, '.github', 'assets', 'readme');
const FONTS = join(REPO, 'brand', 'fonts');

const tokens = JSON.parse(readFileSync(join(REPO, 'brand', 'tokens', 'tokens.json'), 'utf8'));
const G = tokens.graphite;

const fontCache = {};
// Red Hat ships as variable WOFF2, and fontkitten cannot instance a variable
// WOFF2 (getVariation() reopens it as plain TrueType). The default instance
// (400) is used, and heavier labels get a hairline stroke in their own fill.
const font = (file) => (fontCache[file] ??= openFont(readFileSync(join(FONTS, file))));
const TEXT = () => font('RedHatText-latin.woff2');
const DISPLAY = () => font('RedHatDisplay-latin.woff2');

const n = (v) => +v.toFixed(2);

/** One line of outlined text. Returns { d, width }; (x, y) is the baseline start. */
function text(str, { f, size, x = 0, y = 0, tracking = 0 }) {
  // fontkitten has no shaper; plain advances are enough for these short labels.
  const glyphs = f.glyphsForString(str);
  const s = size / f.unitsPerEm;
  let cx = x;
  let d = '';
  glyphs.forEach((g, i) => {
    const X = (v) => n(cx + v * s);
    const Y = (v) => n(y - v * s);
    for (const { command, args } of g.path.commands) {
      if (command === 'moveTo') d += `M${X(args[0])} ${Y(args[1])}`;
      else if (command === 'lineTo') d += `L${X(args[0])} ${Y(args[1])}`;
      else if (command === 'quadraticCurveTo') d += `Q${X(args[0])} ${Y(args[1])} ${X(args[2])} ${Y(args[3])}`;
      else if (command === 'bezierCurveTo')
        d += `C${X(args[0])} ${Y(args[1])} ${X(args[2])} ${Y(args[3])} ${X(args[4])} ${Y(args[5])}`;
      else if (command === 'closePath') d += 'Z';
    }
    cx += g.advanceWidth * s + (i < glyphs.length - 1 ? tracking * size : 0);
  });
  return { d, width: cx - x };
}

/* Themes ------------------------------------------------------------------ */

// GitHub's own page colours sit behind the art: #0d1117 (dark), #ffffff (light).
const THEMES = {
  dark: {
    btnTop: G[800], btnBottom: G[950], edgeTop: G[700], edgeBottom: G[900], sheen: 'rgba(255,255,255,0.10)',
    inkTop: '#FFFFFF', inkBottom: G[300], dim: 0.38, soon: G[500],
  },
  light: {
    btnTop: '#FFFFFF', btnBottom: G[100], edgeTop: G[200], edgeBottom: G[300], sheen: 'rgba(255,255,255,0.9)',
    inkTop: G[700], inkBottom: tokens.ink, dim: 0.34, soon: G[500],
  },
};

const defs = (id, t, h) => `<defs>
  <linearGradient id="${id}-fill" x1="0" y1="0" x2="0" y2="${h}" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="${t.btnTop}"/><stop offset="1" stop-color="${t.btnBottom}"/></linearGradient>
  <linearGradient id="${id}-edge" x1="0" y1="0" x2="0" y2="${h}" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="${t.edgeTop}"/><stop offset="1" stop-color="${t.edgeBottom}"/></linearGradient>
  <linearGradient id="${id}-ink" x1="0" y1="0" x2="0" y2="${h}" gradientUnits="userSpaceOnUse"><stop offset="0.2" stop-color="${t.inkTop}"/><stop offset="0.85" stop-color="${t.inkBottom}"/></linearGradient>
</defs>`;

/* Icons (24 x 24, filled, monochrome) ------------------------------------- */

// Each returns SVG drawn in a 24-unit box; `ink` is the fill.
const ICONS = {
  // The Boltget pixel arrow (brand/svg/mark.svg: a 3-wide shaft of 4 rows,
  // then rows of 9, 7, 5, 3 and 1) on a 24 grid.
  download: (ink) => {
    const rows = [[3, 5], [3, 5], [3, 5], [3, 5], [0, 8], [1, 7], [2, 6], [3, 5], [4, 4]];
    const step = 2.4, cell = 2.05, o = 1.2;
    let d = '';
    rows.forEach(([a, b], r) => {
      for (let col = a; col <= b; col++) d += `M${n(o + col * step)} ${n(o + r * step)}h${cell}v${cell}h-${cell}z`;
    });
    return `<path fill="${ink}" d="${d}"/>`;
  },
  globe: (ink) =>
    `<g fill="none" stroke="${ink}" stroke-width="1.9" stroke-linecap="round"><circle cx="12" cy="12" r="9.2"/><path d="M2.8 12h18.4M12 2.8c2.6 2.6 3.8 5.7 3.8 9.2s-1.2 6.6-3.8 9.2c-2.6-2.6-3.8-5.7-3.8-9.2s1.2-6.6 3.8-9.2z"/></g>`,
  book: (ink) =>
    `<g fill="none" stroke="${ink}" stroke-width="1.9" stroke-linejoin="round"><path d="M12 6.2C10 4.6 7 4 3.5 4.2v14.6c3.5-.2 6.5.4 8.5 2 2-1.6 5-2.2 8.5-2V4.2C17 4 14 4.6 12 6.2z"/><path d="M12 6.2v14.6"/></g>`,
  discord: (ink, id) =>
    `<mask id="${id}-dm"><rect width="24" height="24" fill="#fff"/><ellipse cx="9" cy="12.6" rx="1.75" ry="2" fill="#000"/><ellipse cx="15" cy="12.6" rx="1.75" ry="2" fill="#000"/></mask>
<path mask="url(#${id}-dm)" fill="${ink}" d="M19.3 5.4A17 17 0 0 0 15 4.1l-.55 1.1a15.6 15.6 0 0 0-4.9 0L9 4.1a17 17 0 0 0-4.3 1.3C2 9.5 1.3 13.5 1.6 17.4a17.3 17.3 0 0 0 5.3 2.6l1.15-1.75a11 11 0 0 1-1.8-.85l.45-.35a12.3 12.3 0 0 0 10.6 0l.45.35c-.57.33-1.17.62-1.8.85L17.1 20a17.2 17.2 0 0 0 5.3-2.6c.4-4.5-.7-8.5-3.1-12z"/>`,
  windows: (ink) => `<path fill="${ink}" d="M2 2h9.4v9.4H2zM12.6 2H22v9.4h-9.4zM2 12.6h9.4V22H2zM12.6 12.6H22V22h-9.4z"/>`,
  apple: (ink) =>
    `<path fill="${ink}" d="M12.152 6.896c-.948 0-2.415-1.078-3.96-1.04-2.04.027-3.91 1.183-4.961 3.014-2.117 3.675-.546 9.103 1.519 12.09 1.013 1.454 2.208 3.09 3.792 3.039 1.52-.065 2.09-.987 3.935-.987 1.831 0 2.35.987 3.96.948 1.637-.026 2.676-1.48 3.676-2.948 1.156-1.688 1.636-3.325 1.662-3.415-.039-.013-3.182-1.221-3.22-4.857-.026-3.04 2.48-4.494 2.597-4.559-1.429-2.09-3.623-2.324-4.39-2.376-2-.156-3.675 1.09-4.61 1.09zM15.53 3.83c.843-1.012 1.4-2.427 1.245-3.83-1.207.052-2.662.805-3.532 1.818-.78.896-1.454 2.338-1.273 3.714 1.338.104 2.715-.688 3.559-1.701"/>`,
  // Tux as a silhouette: body and feet filled, belly and eyes cut out.
  linux: (ink, id) =>
    `<mask id="${id}-tm"><rect width="24" height="24" fill="#fff"/><path fill="#000" d="M12 10.2c-1.9 0-3.4 2.3-3.4 5.1 0 1.8.6 3.4 1.6 4.5h3.6c1-1.1 1.6-2.7 1.6-4.5 0-2.8-1.5-5.1-3.4-5.1z"/><ellipse cx="10.55" cy="5.5" rx="1" ry="1.25" fill="#000"/><ellipse cx="13.45" cy="5.5" rx="1" ry="1.25" fill="#000"/></mask>
<path mask="url(#${id}-tm)" fill="${ink}" d="M12 1.2c-2.7 0-4.2 2.1-4.2 4.8 0 1.4.2 2.2-.8 3.6C5.5 11.6 4.2 13.6 4.2 16.3c0 1.6.5 2.9 1.4 3.8h12.8c.9-.9 1.4-2.2 1.4-3.8 0-2.7-1.3-4.7-2.8-6.7-1-1.4-.8-2.2-.8-3.6 0-2.7-1.5-4.8-4.2-4.8z"/>
<path fill="${ink}" d="M3.2 19.9c-.45.7 0 1.6.9 2 1.8.9 4.3 1.1 5.8.5.8-.35.9-1.1.35-1.8-1-1.1-2.7-2.1-4.5-2.1-1.1 0-2.1.6-2.55 1.4zm17.6 0c.45.7 0 1.6-.9 2-1.8.9-4.3 1.1-5.8.5-.8-.35-.9-1.1-.35-1.8 1-1.1 2.7-2.1 4.5-2.1 1.1 0 2.1.6 2.55 1.4z"/>`,
  // A browser-agnostic mark for the extension: a puzzle piece.
  extension: (ink) =>
    `<path fill="${ink}" d="M9.5 2.5a2.5 2.5 0 0 1 2.5 2.5v1h4a2 2 0 0 1 2 2v4h1a2.5 2.5 0 0 1 0 5h-1v3a2 2 0 0 1-2 2h-3.2v-1.3a2.3 2.3 0 0 0-4.6 0V22H5a2 2 0 0 1-2-2v-3.4h1.2a2.3 2.3 0 0 0 0-4.6H3V8a2 2 0 0 1 2-2h2V5a2.5 2.5 0 0 1 2.5-2.5z"/>`,
};

const icon = (name, { x, y, size, ink, id }) =>
  `<g transform="translate(${n(x)} ${n(y)}) scale(${n(size / 24)})">${ICONS[name](ink, id)}</g>`;

/* Pieces ------------------------------------------------------------------ */

/** A link button: icon and a tracked, upper-case label on a soft key. */
function button(name, label, themeName) {
  const t = THEMES[themeName];
  const id = `b-${name}`;
  const H = 34, padX = 13, iconSize = 15, gap = 8;
  const label_ = text(label.toUpperCase(), { f: TEXT(650), size: 11, tracking: 0.12 });
  const W = Math.ceil(padX + iconSize + gap + label_.width + padX);
  const ink = `url(#${id}-ink)`;
  const lbl = text(label.toUpperCase(), { f: TEXT(650), size: 11, x: padX + iconSize + gap, y: H / 2 + 3.9, tracking: 0.12 });
  return {
    width: W,
    svg: `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" role="img" aria-label="${label}">
${defs(id, t, H)}
<rect x="0.5" y="0.5" width="${W - 1}" height="${H - 1}" rx="9" fill="url(#${id}-fill)" stroke="url(#${id}-edge)"/>
<path d="M9.5 1.5h${W - 19}" stroke="${t.sheen}" stroke-linecap="round"/>
${icon(name, { x: padX, y: (H - iconSize) / 2, size: iconSize, ink, id })}
<path fill="${ink}" stroke="${ink}" stroke-width="0.45" stroke-linejoin="round" d="${lbl.d}"/>
</svg>`,
  };
}

/** The platform row: logo + name for each, the ones still to come dimmed with "soon". */
function platforms(themeName) {
  const t = THEMES[themeName];
  const id = 'pf';
  const H = 56, gapItem = 44, logo = 26, gapLogo = 10, size = 25;
  const list = [
    { icon: 'windows', name: 'Windows' },
    { icon: 'extension', name: 'Chrome' },
    { icon: 'apple', name: 'macOS', soon: true },
    { icon: 'linux', name: 'Linux', soon: true },
  ];
  let x = 2;
  const parts = [];
  for (const p of list) {
    const word = text(p.name, { f: DISPLAY(500), size, tracking: -0.02 });
    const baseline = 33;
    const w = text(p.name, { f: DISPLAY(500), size, x: x + logo + gapLogo, y: baseline, tracking: -0.02 });
    let g = `${icon(p.icon, { x, y: baseline - 21.5, size: logo, ink: `url(#${id}-ink)`, id: `${id}-${p.icon}` })}<path fill="url(#${id}-ink)" stroke="url(#${id}-ink)" stroke-width="0.5" stroke-linejoin="round" d="${w.d}"/>`;
    const right = x + logo + gapLogo + word.width;
    if (p.soon) {
      const s = text('SOON', { f: TEXT(650), size: 8.5, tracking: 0.16 });
      const sx = x + logo + gapLogo + (word.width - s.width) / 2;
      const soon = text('SOON', { f: TEXT(650), size: 8.5, x: sx, y: baseline + 15, tracking: 0.16 });
      g = `<g opacity="${t.dim}">${g}</g><path fill="${t.soon}" stroke="${t.soon}" stroke-width="0.35" stroke-linejoin="round" d="${soon.d}"/>`;
    }
    parts.push(g);
    x = right + gapItem;
  }
  const W = Math.ceil(x - gapItem + 2);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" role="img" aria-label="Windows and Chrome today; macOS and Linux soon">
<defs><linearGradient id="${id}-ink" x1="0" y1="8" x2="0" y2="36" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="${t.inkTop}"/><stop offset="1" stop-color="${t.inkBottom}"/></linearGradient></defs>
${parts.join('\n')}
</svg>`;
}

/** A small stack pill: a coloured dot and a tracked label. */
function pill(label, color, themeName) {
  const t = THEMES[themeName];
  const id = `p-${label.toLowerCase()}`;
  const H = 28, padX = 11, dot = 7, gap = 8;
  const probe = text(label.toUpperCase(), { f: TEXT(650), size: 10, tracking: 0.14 });
  const W = Math.ceil(padX + dot + gap + probe.width + padX);
  const lbl = text(label.toUpperCase(), { f: TEXT(650), size: 10, x: padX + dot + gap, y: H / 2 + 3.6, tracking: 0.14 });
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" role="img" aria-label="${label}">
${defs(id, t, H)}
<radialGradient id="${id}-dot" cx="0.35" cy="0.3" r="0.8"><stop offset="0" stop-color="#fff" stop-opacity="0.55"/><stop offset="0.5" stop-color="${color}" stop-opacity="0"/></radialGradient>
<rect x="0.5" y="0.5" width="${W - 1}" height="${H - 1}" rx="7.5" fill="url(#${id}-fill)" stroke="url(#${id}-edge)"/>
<circle cx="${padX + dot / 2}" cy="${H / 2}" r="${dot / 2}" fill="${color}"/>
<circle cx="${padX + dot / 2}" cy="${H / 2}" r="${dot / 2}" fill="url(#${id}-dot)"/>
<path fill="url(#${id}-ink)" stroke="url(#${id}-ink)" stroke-width="0.4" stroke-linejoin="round" d="${lbl.d}"/>
</svg>`;
}

/* Write ------------------------------------------------------------------- */

rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });
const write = (name, svg) => writeFileSync(join(OUT, name), `${svg.trim()}\n`);

for (const theme of ['dark', 'light']) {
  for (const [name, label] of [
    ['download', 'Download'],
    ['globe', 'Website'],
    ['book', 'Docs'],
    ['discord', 'Discord'],
  ]) {
    write(`button-${label.toLowerCase()}-${theme}.svg`, button(name, label, theme).svg);
  }
  write(`platforms-${theme}.svg`, platforms(theme));
  for (const [label, color] of [
    ['Rust', '#E5704B'],
    ['Tauri', '#FFC131'],
    ['Svelte', '#FF3E00'],
  ]) {
    write(`stack-${label.toLowerCase()}-${theme}.svg`, pill(label, color, theme));
  }
}
console.log(`README art written to ${OUT}`);
