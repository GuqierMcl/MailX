import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve, sep } from "node:path";
import { spawnSync } from "node:child_process";
import { deflateSync } from "node:zlib";
import { Resvg } from "@resvg/resvg-js";

const root = resolve(import.meta.dirname, "..");
const check = process.argv.includes("--check");
if (process.argv.slice(2).some((arg) => arg !== "--check")) {
  throw new Error("Usage: bun scripts/generate-icons.mjs [--check]");
}
const master = readFileSync(join(root, "branding/logo.svg"), "utf8");
const artwork = master.replace(/^[\s\S]*?<svg\b[^>]*>/, "").replace(/<\/svg>\s*$/, "")
  .replace(/<(title|desc)\b[^>]*>[\s\S]*?<\/\1>/g, "").trim();
const glyph = artwork.match(/<g\b[\s\S]*<\/g>/)?.[0];
if (!glyph || !/viewBox="0 0 1024 1024"/.test(master)) {
  throw new Error("branding/logo.svg must retain the 1024-square canvas and a separate glyph group.");
}
const font = { loadSystemFonts: false, fontFiles: [join(root, "branding/fonts/MailXBrandText.ttf")] };
const outputs = new Map();
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const put = (path, bytes) => outputs.set(path, Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes));
const svg = (width, height, body, viewBox = `0 0 ${width} ${height}`) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="${viewBox}">${body}</svg>\n`;
const nested = (body, x, y, size) => `<svg x="${x}" y="${y}" width="${size}" height="${size}" viewBox="0 0 1024 1024">${body}</svg>`;
const scaledGlyph = (scale = 1, color = "#fff") =>
  `<g transform="translate(512 512) scale(${scale}) translate(-512 -512)">${glyph.replaceAll("#fff", color)}</g>`;
const png = (source, size) => new Resvg(source, { font, fitTo: { mode: "width", value: size } }).render().asPng();
const outline = (source) => new Resvg(source, { font }).toString().trimEnd() + "\n";
const text = (value, x, y, size, color = "#111") =>
  `<text x="${x}" y="${y}" font-family="MailX Brand Text" font-size="${size}" fill="${color}">${value}</text>`;

// iOS assets must have no alpha channel, even when every alpha sample is 255.
const crcTable = Array.from({ length: 256 }, (_, index) => {
  let value = index;
  for (let bit = 0; bit < 8; bit++) value = value & 1 ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
  return value >>> 0;
});
function pngChunk(type, data) {
  const bytes = Buffer.alloc(data.length + 12);
  bytes.writeUInt32BE(data.length, 0);
  bytes.write(type, 4, 4, "ascii");
  data.copy(bytes, 8);
  let crc = 0xffffffff;
  for (const byte of bytes.subarray(4, -4)) crc = crcTable[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  bytes.writeUInt32BE((crc ^ 0xffffffff) >>> 0, bytes.length - 4);
  return bytes;
}
function opaquePng(source, size) {
  const rendered = new Resvg(source, { font, fitTo: { mode: "width", value: size } }).render();
  const { width, height, pixels } = rendered;
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8;
  header[9] = 2; // Truecolor RGB.
  const rows = Buffer.alloc(height * (1 + width * 3));
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const pixel = (y * width + x) * 4;
      if (pixels[pixel + 3] !== 255) throw new Error("Opaque icon source contains transparency");
      pixels.copy(rows, y * (width * 3 + 1) + 1 + x * 3, pixel, pixel + 3);
    }
  }
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), pngChunk("IHDR", header),
    pngChunk("IDAT", deflateSync(rows, { level: 9 })), pngChunk("IEND", Buffer.alloc(0))]);
}

// The native ICNS writer iterates an unordered map. Canonicalize chunk order
// without changing image contents, so regeneration and --check are stable.
function canonicalIcns(bytes) {
  if (bytes.toString("ascii", 0, 4) !== "icns" || bytes.readUInt32BE(4) !== bytes.length) {
    throw new Error("Invalid ICNS header");
  }
  const chunks = [];
  for (let offset = 8; offset < bytes.length;) {
    const length = bytes.readUInt32BE(offset + 4);
    if (length < 8 || offset + length > bytes.length) throw new Error("Invalid ICNS chunk");
    const chunk = bytes.subarray(offset, offset + length);
    if (chunk.toString("ascii", 0, 4) === "TOC ") throw new Error("Update the ICNS normalizer for the new Tauri TOC format");
    chunks.push(chunk);
    offset += length;
  }
  chunks.sort((a, b) => Buffer.compare(a.subarray(0, 4), b.subarray(0, 4)));
  return Buffer.concat([bytes.subarray(0, 8), ...chunks]);
}

// Outline the wordmark with the bundled font, so exported SVGs need no fonts.
function wordmark(color, size = 208) {
  const rendered = new Resvg(svg(1200, 360, text("MailX", 20, 280, size, color)), { font });
  const bbox = rendered.innerBBox();
  if (!bbox || bbox.width < 100) throw new Error("The bundled branding font did not load.");
  const body = rendered.toString().replace(/^<svg\b[^>]*>/, "").replace(/<\/svg>\s*$/, "");
  return {
    body: `<g transform="translate(${-bbox.x} ${-bbox.y})">${body}</g>`,
    width: Math.ceil(bbox.width), height: Math.ceil(bbox.height),
  };
}

const square = svg(1024, 1024, `<rect width="1024" height="1024" fill="#000"/>${glyph}`);
const maskable = svg(1024, 1024, `<rect width="1024" height="1024" fill="#000"/>${scaledGlyph(0.86)}`);
const favicon = svg(64, 64, artwork, "64 64 896 896");
const marks = {};
const lockups = {};
put("branding/variants/app-square.svg", square);
put("branding/variants/app-maskable.svg", maskable);
for (const [name, color] of [["black", "#000"], ["white", "#fff"]]) {
  marks[name] = svg(1024, 1024, scaledGlyph(1.42, color));
  put(`branding/variants/mark-${name}.svg`, marks[name]);
  const word = wordmark(color);
  const wordSvg = svg(word.width + 32, word.height + 32, `<g transform="translate(16 16)">${word.body}</g>`);
  put(`branding/variants/wordmark-${name}.svg`, wordSvg);
  for (const width of [512, 1024]) put(`branding/png/wordmark-${name}-${width}.png`, png(wordSvg, width));
  lockups[name] = svg(360 + word.width + 24, 320,
    `${nested(artwork, 0, 0, 320)}<g transform="translate(360 ${(320 - word.height) / 2})">${word.body}</g>`);
  put(`branding/variants/logo-horizontal-${name}.svg`, lockups[name]);
  for (const width of [512, 1024, 2048]) put(`branding/png/logo-horizontal-${name}-${width}.png`, png(lockups[name], width));
  const stackedWord = wordmark(color, 156);
  const stacked = svg(640, 760, `${nested(artwork, 64, 16, 512)}<g transform="translate(${(640 - stackedWord.width) / 2} 576)">${stackedWord.body}</g>`);
  put(`branding/variants/logo-stacked-${name}.svg`, stacked);
  for (const width of [640, 1280]) put(`branding/png/logo-stacked-${name}-${width}.png`, png(stacked, width));
  for (const size of [64, 128, 256, 512, 1024]) put(`branding/png/mark-${name}-${size}.png`, png(marks[name], size));
}
for (const size of [16, 24, 32, 48, 64, 96, 128, 192, 256, 512, 1024, 2048]) {
  put(`branding/png/app-${size}.png`, png(master, size));
}

// Favicon ICO contains multiple actual resolutions, not a renamed PNG.
function ico(source, sizes) {
  const images = sizes.map((size) => png(source, size));
  const header = Buffer.alloc(6 + sizes.length * 16);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(sizes.length, 4);
  let offset = header.length;
  sizes.forEach((size, index) => {
    const entry = 6 + index * 16;
    header[entry] = size === 256 ? 0 : size;
    header[entry + 1] = header[entry];
    header.writeUInt16LE(1, entry + 4);
    header.writeUInt16LE(32, entry + 6);
    header.writeUInt32LE(images[index].length, entry + 8);
    header.writeUInt32LE(offset, entry + 12);
    offset += images[index].length;
  });
  return Buffer.concat([header, ...images]);
}

put("frontend/public/logo.svg", master);
put("frontend/public/favicon.svg", favicon);
put("frontend/public/favicon.ico", ico(favicon, [16, 32, 48]));
for (const size of [16, 32, 48]) put(`frontend/public/favicon-${size}x${size}.png`, png(favicon, size));
put("frontend/public/apple-touch-icon.png", opaquePng(square, 180));
put("frontend/public/safari-pinned-tab.svg", marks.black);
for (const size of [192, 512]) {
  put(`frontend/public/pwa/icon-${size}.png`, png(master, size));
  put(`frontend/public/pwa/maskable-${size}.png`, png(maskable, size));
}
put("frontend/public/site.webmanifest", JSON.stringify({
  name: "MailX", short_name: "MailX", id: "./", start_url: "./", scope: "./",
  display: "standalone", background_color: "#ffffff", theme_color: "#000000",
  icons: [192, 512].flatMap((size) => [
    { src: `pwa/icon-${size}.png`, sizes: `${size}x${size}`, type: "image/png", purpose: "any" },
    { src: `pwa/maskable-${size}.png`, sizes: `${size}x${size}`, type: "image/png", purpose: "maskable" },
  ]),
}, null, 2) + "\n");

// Tray assets are prepared only; this does not enable a running system tray.
put("desktop/icons/icon.svg", master);
for (const name of ["black", "white"]) {
  put(`desktop/icons/tray/tray-${name}.svg`, marks[name]);
  for (const size of [16, 20, 24, 32, 40, 48, 64]) put(`desktop/icons/tray/tray-${name}-${size}.png`, png(marks[name], size));
}
put("desktop/icons/tray/trayTemplate.png", png(marks.black, 22));
put("desktop/icons/tray/trayTemplate@2x.png", png(marks.black, 44));

function shareCard(width, height, light = false) {
  const foreground = light ? "#111" : "#fff";
  const word = wordmark(foreground, 188);
  const center = height / 2;
  return outline(svg(width, height,
    `<rect width="${width}" height="${height}" fill="${light ? "#f5f5f3" : "#121416"}"/>` +
    `<path d="M64 ${height - 76}H${width - 64}" stroke="${light ? "#d5d6d4" : "#35383b"}"/>` +
    nested(artwork, 60, center - 190, 320) +
    `<g transform="translate(424 ${center - 110})">${word.body}</g>` +
    text("Fast. Compatible. Local-first.", 426, center + 90, 30, light ? "#62676b" : "#b7bdc2") +
    text("Mail workspace", 72, height - 38, 20, foreground) +
    text("MailX", width - 140, height - 38, 20, foreground)));
}
for (const [name, width, height] of [["open-graph", 1200, 630], ["github-social", 1280, 640]]) {
  for (const tone of ["dark", "light"]) {
    const card = shareCard(width, height, tone === "light");
    put(`branding/social/${name}-${tone}.svg`, card);
    put(`branding/social/${name}-${tone}.png`, png(card, width));
  }
}
// A full-bleed avatar also works when the receiving site clips it to a circle.
put("branding/social/avatar.svg", maskable);
for (const size of [400, 1024]) put(`branding/social/avatar-${size}.png`, png(maskable, size));

// Use the installed Tauri CLI directly: `tauri icon` has no --config option.
const requireFrontend = createRequire(join(root, "frontend/package.json"));
const cli = join(dirname(requireFrontend.resolve("@tauri-apps/cli/package.json")), "tauri.js");
const temporary = mkdtempSync(join(tmpdir(), "mailx-icons-"));
function files(path) {
  return readdirSync(path, { withFileTypes: true }).flatMap((entry) =>
    entry.isDirectory() ? files(join(path, entry.name)) : [join(path, entry.name)]);
}
function tauriIcons(input, output) {
  const result = spawnSync(process.execPath, [cli, "icon", input, "--output", output, "--ios-color", "#000000"], {
    cwd: root, encoding: "utf8", maxBuffer: 8 * 1024 * 1024,
  });
  if (result.error || result.status !== 0) throw new Error(result.error?.message ?? result.stderr ?? "Tauri icon generation failed");
}
try {
  writeFileSync(join(temporary, "app.svg"), master);
  writeFileSync(join(temporary, "square.svg"), square);
  writeFileSync(join(temporary, "background.svg"), svg(1024, 1024, '<rect width="1024" height="1024" fill="#000"/>'));
  writeFileSync(join(temporary, "foreground.svg"), svg(1024, 1024, scaledGlyph(0.72)));
  writeFileSync(join(temporary, "mobile.json"), JSON.stringify({
    default: "square.svg", bg_color: "#000000", android_bg: "background.svg",
    android_fg: "foreground.svg", android_fg_scale: 100, android_monochrome: "foreground.svg",
  }));
  const desktopOutput = join(temporary, "desktop");
  const mobileOutput = join(temporary, "mobile");
  tauriIcons(join(temporary, "app.svg"), desktopOutput);
  tauriIcons(join(temporary, "mobile.json"), mobileOutput);
  for (const file of files(desktopOutput)) {
    const path = relative(desktopOutput, file).split(sep).join("/");
    if (!path.includes("/")) {
      const data = readFileSync(file);
      put(`desktop/icons/${path}`, path.endsWith(".icns") ? canonicalIcns(data) : data);
    }
  }
  for (const file of files(mobileOutput)) {
    const path = relative(mobileOutput, file).split(sep).join("/");
    if (path.startsWith("ios/") || path.startsWith("android/")) {
      const data = readFileSync(file);
      put(`branding/mobile/${path}`, path.startsWith("ios/") && path.endsWith(".png")
        ? opaquePng(square, data.readUInt32BE(16)) : data);
    }
  }
} finally {
  if (dirname(temporary) !== resolve(tmpdir()) || !temporary.startsWith(join(tmpdir(), "mailx-icons-"))) {
    throw new Error("Refusing to clean an unexpected temporary directory");
  }
  rmSync(temporary, { recursive: true, force: true });
}

// Standalone review sheet: the assets themselves stay transparent where appropriate.
let preview = `<rect width="1200" height="1140" fill="#eff0f0"/>` +
  text("MailX", 48, 67, 40) + text("Brand assets / 2026", 50, 100, 18, "#666c72");
preview += `<rect x="32" y="130" width="552" height="364" rx="20" fill="#fff"/>` +
  `<rect x="616" y="130" width="552" height="364" rx="20" fill="#15181b"/>`;
preview += text("Primary app icon", 56, 169, 18) + nested(artwork, 180, 193, 256);
preview += text("Monochrome marks", 640, 169, 18, "#fff") +
  nested(scaledGlyph(1.42), 668, 226, 172) +
  `<rect x="920" y="226" width="172" height="172" rx="12" fill="#fff"/>` + nested(scaledGlyph(1.42, "#000"), 920, 226, 172);
for (const [name, y, bg] of [["black", 518, "#fff"], ["white", 710, "#15181b"]]) {
  const content = lockups[name].replace(/^<svg\b[^>]*>/, "").replace(/<\/svg>\s*$/, "");
  const width = Number(lockups[name].match(/width="([\d.]+)"/)[1]);
  preview += `<rect x="32" y="${y}" width="1136" height="168" rx="16" fill="${bg}"/>` +
    `<svg x="64" y="${y + 4}" width="600" height="160" viewBox="0 0 ${width} 320">${content}</svg>` +
    text(name === "black" ? "For light backgrounds" : "For dark backgrounds", 792, y + 91, 19, name === "black" ? "#666c72" : "#aeb6bd");
}
preview += text("Small sizes", 48, 935, 18);
for (const [index, size] of [16, 24, 32, 48, 64, 96].entries()) {
  const x = 84 + index * 165;
  preview += nested(artwork, x - size / 2, 1012 - size / 2, size) + text(`${size} px`, x - 24, 1093, 15, "#666c72");
}
put("branding/preview.png", png(svg(1200, 1140, preview), 1200));

const inventory = {
  source: "branding/logo.svg", sourceSha256: hash(Buffer.from(master)),
  fontSha256: hash(readFileSync(font.fontFiles[0])),
  generator: "scripts/generate-icons.mjs",
  files: [...outputs.entries()].sort(([a], [b]) => a.localeCompare(b, "en")).map(([path, bytes]) => ({
    path, bytes: bytes.length, sha256: hash(bytes),
    ...(path.endsWith(".png") ? { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) } : {}),
  })),
};
put("branding/generated-assets.json", JSON.stringify(inventory, null, 2) + "\n");
const mismatches = [];
for (const [path, bytes] of outputs) {
  const target = resolve(root, path);
  if (!target.startsWith(root + sep)) throw new Error(`Output is outside the repository: ${path}`);
  if (check) {
    if (!existsSync(target) || !readFileSync(target).equals(bytes)) mismatches.push(path);
  } else {
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, bytes);
  }
}
if (mismatches.length) {
  console.error(`Generated assets are missing or stale. Run bun run icons:generate:\n${mismatches.join("\n")}`);
  process.exit(1);
}
console.log(`${check ? "Verified" : "Generated"} ${outputs.size} files from branding/logo.svg.`);
