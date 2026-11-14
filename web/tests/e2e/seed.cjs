// Seed fixtures for the Playwright e2e suite: an isolated throwaway HOME with
// a project folder, generated PNG fixtures, and a projects.json the server
// loads at startup. Run BEFORE `npx playwright test` (see make test-e2e / CI).
//
// NOTE: the project id here is the folder path, but POST /projects assigns a
// uuid id, so specs must read the id from the API instead of assuming either.
//
// The root must match playwright.config.ts: os.tmpdir() is NOT usable because
// sandboxed/ephemeral TMPDIRs differ between processes. Keep the default and
// the PS_E2E_HOME override identical in both files.
"use strict";

const fs = require("node:fs");
const path = require("node:path");
const zlib = require("node:zlib");

const HOME = process.env.PS_E2E_HOME || "/tmp/patternstudio-e2e-home";
const PROJECT = path.join(HOME, "ps-e2e-project");

// --- minimal 24-bit RGB PNG encoder ----------------------------------------

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

/** Encode an RGB image; pixel(x, y) returns [r, g, b] in 0..255. */
function encodePng(width, height, pixel) {
  const raw = Buffer.alloc(height * (1 + width * 3));
  let o = 0;
  for (let y = 0; y < height; y++) {
    raw[o++] = 0; // filter: none
    for (let x = 0; x < width; x++) {
      const [r, g, b] = pixel(x, y);
      raw[o++] = r;
      raw[o++] = g;
      raw[o++] = b;
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // color type: truecolor
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", zlib.deflateSync(raw, { level: 6 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// --- fixtures ----------------------------------------------------------------

// color.png: 96x96, a 3x3 grid of solid colors. Small and few-colors so even
// a debug binary traces it in seconds: the workflow spec waits on its layers.
const PALETTE = [
  [214, 40, 40],
  [40, 180, 90],
  [45, 90, 210],
  [235, 200, 50],
  [240, 240, 240],
  [240, 130, 40],
  [150, 60, 180],
  [50, 170, 200],
  [60, 60, 60],
];
const colorPng = encodePng(96, 96, (x, y) =>
  PALETTE[Math.floor(x / 32) + 3 * Math.floor(y / 32)],
);

// photo.png: 256x256 xorshift noise. Color segmentation on per-pixel noise is
// slow in a debug binary (seconds), which is exactly what the busy-window test
// needs: the second background trace must land while the first is in flight.
function xorshift32(seed) {
  let s = seed >>> 0 || 1;
  return () => {
    s ^= (s << 13) >>> 0;
    s ^= s >>> 17;
    s ^= (s << 5) >>> 0;
    return s >>> 0;
  };
}
const rnd = xorshift32(0xb16b00b5);
// Keep the RNG advancing per pixel (not per channel) so colors stay vivid.
const noisePng = encodePng(256, 256, () => {
  const v = rnd();
  return [(v & 255), ((v >>> 8) & 255), ((v >>> 16) & 255)];
});

// junk.png: valid PNG header checksum-wise nothing; decoders reject it outright
// so the server answers code=invalid_input and the UI shows "Invalid image:".
const junkPng = Buffer.from("this is not a png, just junk bytes for the e2e suite");

// --- layout ------------------------------------------------------------------

fs.rmSync(HOME, { recursive: true, force: true });
fs.mkdirSync(path.join(HOME, ".patternstudio"), { recursive: true });
fs.mkdirSync(PROJECT, { recursive: true });

fs.writeFileSync(path.join(PROJECT, "color.png"), colorPng);
fs.writeFileSync(path.join(PROJECT, "photo.png"), noisePng);
fs.writeFileSync(path.join(PROJECT, "junk.png"), junkPng);

fs.writeFileSync(
  path.join(HOME, ".patternstudio", "projects.json"),
  JSON.stringify({
    projects: [
      {
        id: PROJECT,
        path: PROJECT,
        name: "e2e",
        images: ["color.png", "junk.png", "photo.png"],
      },
    ],
  }, null, 2) + "\n",
);

console.log(`seeded ${PROJECT} (HOME=${HOME})`);
