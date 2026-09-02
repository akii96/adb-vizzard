// Generates the app icons Tauri needs, with no image-library dependency.
// Draws a rounded dark tile with three ascending bars (a benchmark sweep motif)
// and emits the PNG sizes plus a PNG-payload .ico.
//
//   node scripts/gen-icons.mjs

import { deflateSync } from "node:zlib";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { execSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const OUT_DIR = join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri", "icons");

const CRC_TABLE = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c;
  }
  return table;
})();

function crc32(buf) {
  let c = -1;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

function encodePng(size, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // colour type: RGBA
  // 10..12 stay zero: deflate, adaptive filtering, no interlace.

  // Prefix each scanline with filter type 0.
  const raw = Buffer.alloc((size * 4 + 1) * size);
  for (let y = 0; y < size; y++) {
    const rowStart = y * (size * 4 + 1);
    raw[rowStart] = 0;
    rgba.copy(raw, rowStart + 1, y * size * 4, (y + 1) * size * 4);
  }

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const lerp = (a, b, t) => a + (b - a) * t;
const clamp01 = (v) => (v < 0 ? 0 : v > 1 ? 1 : v);

// Signed distance to a rounded rectangle, used for antialiased edges.
function roundedRectCoverage(px, py, x0, y0, x1, y1, radius) {
  const cx = Math.max(x0 + radius, Math.min(px, x1 - radius));
  const cy = Math.max(y0 + radius, Math.min(py, y1 - radius));
  const dx = px - cx;
  const dy = py - cy;
  const dist = Math.sqrt(dx * dx + dy * dy);
  // Inside the straight-edge core the distance is 0, so coverage is full.
  return clamp01(radius - dist + 0.5);
}

function over(dst, i, r, g, b, a) {
  if (a <= 0) return;
  const inv = 1 - a;
  dst[i] = Math.round(r * a + dst[i] * inv);
  dst[i + 1] = Math.round(g * a + dst[i + 1] * inv);
  dst[i + 2] = Math.round(b * a + dst[i + 2] * inv);
  dst[i + 3] = Math.round(255 * a + dst[i + 3] * inv);
}

function render(size) {
  const rgba = Buffer.alloc(size * size * 4, 0);
  const s = size / 128; // design is authored at 128px

  const tile = { x0: 6 * s, y0: 6 * s, x1: size - 6 * s, y1: size - 6 * s, r: 26 * s };

  // Bars: [xStart, width, topY] in design units, drawn bottom-anchored.
  const baseY = 100;
  const bars = [
    { x: 30, w: 15, top: 74, c0: [56, 189, 248], c1: [14, 165, 233] },
    { x: 56, w: 15, top: 52, c0: [45, 212, 191], c1: [16, 185, 129] },
    { x: 82, w: 15, top: 28, c0: [251, 191, 36], c1: [245, 158, 11] },
  ];

  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const i = (y * size + x) * 4;
      const px = x + 0.5;
      const py = y + 0.5;

      const tileA = roundedRectCoverage(px, py, tile.x0, tile.y0, tile.x1, tile.y1, tile.r);
      if (tileA > 0) {
        // Diagonal slate gradient for the tile body.
        const t = clamp01((px + py) / (size * 2));
        over(rgba, i, lerp(30, 15, t), lerp(41, 23, t), lerp(59, 42, t), tileA);
      }

      for (const bar of bars) {
        const bx0 = bar.x * s;
        const bx1 = (bar.x + bar.w) * s;
        const by0 = bar.top * s;
        const by1 = baseY * s;
        const barA = roundedRectCoverage(px, py, bx0, by0, bx1, by1, 5 * s) * tileA;
        if (barA > 0) {
          const t = clamp01((py - by0) / (by1 - by0));
          over(
            rgba,
            i,
            lerp(bar.c0[0], bar.c1[0], t),
            lerp(bar.c0[1], bar.c1[1], t),
            lerp(bar.c0[2], bar.c1[2], t),
            barA,
          );
        }
      }
    }
  }

  return rgba;
}

function buildIco(pngs) {
  const count = pngs.length;
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(count, 4);

  const dir = Buffer.alloc(16 * count);
  let offset = 6 + 16 * count;
  pngs.forEach(({ size, data }, idx) => {
    const at = idx * 16;
    dir[at] = size >= 256 ? 0 : size; // 0 means 256
    dir[at + 1] = size >= 256 ? 0 : size;
    dir[at + 2] = 0; // palette
    dir[at + 3] = 0; // reserved
    dir.writeUInt16LE(1, at + 4); // colour planes
    dir.writeUInt16LE(32, at + 6); // bits per pixel
    dir.writeUInt32LE(data.length, at + 8);
    dir.writeUInt32LE(offset, at + 12);
    offset += data.length;
  });

  return Buffer.concat([header, dir, ...pngs.map((p) => p.data)]);
}

mkdirSync(OUT_DIR, { recursive: true });

const outputs = {
  "32x32.png": 32,
  "128x128.png": 128,
  "128x128@2x.png": 256,
  "icon.png": 512,
};

const cache = new Map();
const pngFor = (size) => {
  if (!cache.has(size)) cache.set(size, encodePng(size, render(size)));
  return cache.get(size);
};

for (const [name, size] of Object.entries(outputs)) {
  writeFileSync(join(OUT_DIR, name), pngFor(size));
  console.log(`wrote icons/${name} (${size}x${size})`);
}

const ico = buildIco([16, 32, 48, 64, 128, 256].map((size) => ({ size, data: pngFor(size) })));
writeFileSync(join(OUT_DIR, "icon.ico"), ico);
console.log(`wrote icons/icon.ico (${ico.length} bytes)`);

// macOS: build icon.icns from the rendered PNGs using sips + iconutil (both macOS built-ins).
if (process.platform === "darwin") {
  const iconsetDir = join(OUT_DIR, "icon.iconset");
  mkdirSync(iconsetDir, { recursive: true });

  // Each entry: [logical size, scale, filename used by iconutil convention]
  const icnsEntries = [
    [16, 1, "icon_16x16.png"],
    [16, 2, "icon_16x16@2x.png"],
    [32, 1, "icon_32x32.png"],
    [32, 2, "icon_32x32@2x.png"],
    [128, 1, "icon_128x128.png"],
    [128, 2, "icon_128x128@2x.png"],
    [256, 1, "icon_256x256.png"],
    [256, 2, "icon_256x256@2x.png"],
    [512, 1, "icon_512x512.png"],
    [512, 2, "icon_512x512@2x.png"],
  ];

  for (const [logical, scale, name] of icnsEntries) {
    writeFileSync(join(iconsetDir, name), pngFor(logical * scale));
  }

  const icnsPath = join(OUT_DIR, "icon.icns");
  execSync(`iconutil -c icns "${iconsetDir}" -o "${icnsPath}"`);
  console.log(`wrote icons/icon.icns`);
  rmSync(iconsetDir, { recursive: true });
}
