// Composes the store images from the captures of the real UI: a headline in
// Red Hat Display, one quiet line in Red Hat Text and the UI in a soft-shadowed
// frame on the brand's pearl sky (brand/source/sky-master.webp). Each image
// is an HTML page (templates/slide.css) rendered by Chromium at twice its
// size and scaled down, then written without metadata.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "@playwright/test";
import sharp from "sharp";
import { chromiumPath } from "./lib/browser.mjs";
import { CAPTURES, OUTPUT, REPO_DIR, SCALE, STORE_DIR, savePng } from "./lib/paths.mjs";

const dataUri = (path, type) => `data:${type};base64,${readFileSync(path).toString("base64")}`;
const brand = (path) => join(REPO_DIR, "brand", path);
const capture = (name) => dataUri(join(CAPTURES, `${name}.png`), "image/png");

/** The CSS size of a capture (captures are taken at SCALE device pixels per CSS pixel). */
async function size(name) {
  const { width, height } = await sharp(join(CAPTURES, `${name}.png`)).metadata();
  return { w: width / SCALE, h: height / SCALE };
}

const SKY = dataUri(brand("source/sky-master.webp"), "image/webp");
const FONTS = `
@font-face { font-family: "Red Hat Display"; font-weight: 300 900;
  src: url(${dataUri(brand("fonts/RedHatDisplay-latin.woff2"), "font/woff2")}) format("woff2"); }
@font-face { font-family: "Red Hat Text"; font-weight: 300 700;
  src: url(${dataUri(brand("fonts/RedHatText-latin.woff2"), "font/woff2")}) format("woff2"); }`;
const CSS = readFileSync(join(STORE_DIR, "templates", "slide.css"), "utf8");

function page(width, height, body, { sky = "" } = {}) {
  return `<!doctype html><html><head><meta charset="utf-8"><style>${FONTS}
:root { --w: ${width}px; --h: ${height}px; --sky: url(${SKY}); }
${CSS}</style></head><body><div class="slide ${sky}">${body}</div></body></html>`;
}

/** A frame around a capture, `scale` times its CSS size, optionally cropped (CSS px of the capture). */
async function frame(name, { scale = 1, crop, kind = "window", style = "" } = {}) {
  const full = await size(name);
  const c = crop ?? { x: 0, y: 0, w: full.w, h: full.h };
  const w = c.w * scale;
  const h = c.h * scale;
  // An options section brings its own border and corners (8 px, --radius-floating).
  const radius = kind === "card" ? `border-radius:${8 * scale}px;` : "";
  return `<div class="frame ${kind}" style="width:${w}px;height:${h}px;${radius}${style}">
    <img src="${capture(name)}" style="width:${full.w * scale}px;height:${full.h * scale}px;left:${-c.x * scale}px;top:${-c.y * scale}px"></div>`;
}

const words = (title, line, extra = "") =>
  `<div class="words ${extra}"><h1>${title}</h1>${line ? `<p>${line}</p>` : ""}</div>`;

/** The five 1280 x 800 screenshots. */
async function screenshots() {
  return [
    {
      name: "screenshot-1-handoff",
      sky: "sky-a",
      body:
        words(
          "Downloads from your browser go to Boltget",
          "At full speed, with the cookies and headers the site expects.",
          "top",
        ) +
        (await frame("app-running-dark", {
          scale: 0.86,
          style: "position:absolute;left:232px;top:268px",
        })) +
        (await frame("popup-dark", {
          scale: 1,
          kind: "popover",
          style: "position:absolute;left:96px;top:452px",
        })),
    },
    {
      name: "screenshot-2-resume",
      sky: "sky-b",
      body:
        words(
          "Picks up where it stopped",
          "Paused, cut off or restarted: Boltget resumes from the bytes it already has.",
          "side",
        ) +
        (await frame("app-paused-pane-dark", {
          scale: 1.45,
          crop: { x: 0, y: 0, w: 360, h: 360 },
          style: "position:absolute;right:96px;top:139px",
        })),
    },
    {
      name: "screenshot-3-rules",
      sky: "sky-c",
      body:
        words(
          "Your rules: what to catch, what to leave",
          "File types, a smallest size, and the sites that always or never go to Boltget.",
          "side narrow",
        ) +
        (await frame("options-what-goes-to-boltget-light", {
          scale: 0.92,
          kind: "card",
          style: "position:absolute;right:96px;top:68px",
        })) +
        (await frame("options-sites-light", {
          scale: 0.92,
          kind: "card",
          style: "position:absolute;right:96px;top:440px",
        })),
    },
    {
      name: "screenshot-4-toolbar",
      sky: "sky-d",
      body:
        words(
          "One click away",
          "Switch capture off, pause it for ten minutes, or right-click a link and choose Download with Boltget.",
          "side",
        ) +
        (await frame("popup-light", {
          scale: 1.55,
          kind: "popover",
          style: "position:absolute;right:150px;top:198px",
        })),
    },
    {
      name: "screenshot-5-private",
      sky: "sky-e",
      body:
        words(
          "Private by design",
          "It talks only to the Boltget app on your computer. No account, no servers, no tracking.",
          "side narrow",
        ) +
        (await frame("options-site-access-light", {
          scale: 1,
          kind: "card",
          style: "position:absolute;right:96px;top:206px",
        })) +
        (await frame("options-connection-light", {
          scale: 1,
          kind: "card",
          style: "position:absolute;right:96px;top:444px",
        })),
    },
  ].map((s) => ({ ...s, width: 1280, height: 800 }));
}

async function tiles() {
  const lockup = dataUri(brand("svg/lockup-horizontal-on-light.svg"), "image/svg+xml");
  return [
    {
      name: "promo-small-440x280",
      width: 440,
      height: 280,
      sky: "sky-tile",
      body: `<div class="tile"><img class="lockup" src="${lockup}" alt="">
        <p>Your browser’s downloads, at full speed.</p></div>`,
    },
    {
      name: "marquee-1400x560",
      width: 1400,
      height: 560,
      sky: "sky-marquee",
      body:
        `<div class="marquee"><img class="lockup" src="${lockup}" alt="">
          <h1>Your browser’s downloads, at full speed</h1>
          <p>Big files go to the Boltget app, which fetches them over many connections and resumes them.</p></div>` +
        (await frame("app-running-dark", {
          scale: 0.7,
          crop: { x: 0, y: 0, w: 1080, h: 680 },
          style: "position:absolute;left:690px;top:96px",
        })),
    },
  ];
}

/** Renders every slide into output/ and returns the files written. */
export async function compose() {
  const browser = await chromium.launch({ executablePath: chromiumPath() });
  const written = [];
  try {
    for (const slide of [...(await screenshots()), ...(await tiles())]) {
      const tab = await browser.newPage({
        viewport: { width: slide.width, height: slide.height },
        deviceScaleFactor: SCALE,
      });
      await tab.setContent(page(slide.width, slide.height, slide.body, { sky: slide.sky }), {
        waitUntil: "load",
      });
      await tab.evaluate(() => document.fonts.ready);
      const png = await tab.screenshot({ type: "png" });
      await tab.close();
      const path = join(OUTPUT, `${slide.name}.png`);
      await savePng(png, path, {
        opaque: true,
        resize: { width: slide.width, height: slide.height },
      });
      written.push(path);
    }
  } finally {
    await browser.close();
  }
  return written;
}

/** The 128 x 128 store icon: the brand's native 96 px Onyx icon with 16 px of clear space. */
export async function storeIcon() {
  const path = join(OUTPUT, "store-icon-128.png");
  const icon = await sharp(brand("png/icon-96.png")).toBuffer();
  const canvas = sharp({
    create: { width: 128, height: 128, channels: 4, background: { r: 0, g: 0, b: 0, alpha: 0 } },
  }).composite([{ input: icon, left: 16, top: 16 }]);
  await savePng(await canvas.png().toBuffer(), path);
  return path;
}
