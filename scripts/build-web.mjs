import { spawnSync } from "node:child_process";
import {
  cpSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { resolve } from "node:path";

const extension = process.argv.includes("--extension");
const out = resolve(extension ? "output/extension" : "output/web");
const result = spawnSync(
  process.execPath,
  [
    "node_modules/vite/bin/vite.js",
    "build",
    "--mode",
    "browser",
    "--base",
    "./",
    "--outDir",
    out,
  ],
  { stdio: "inherit" },
);
if (result.status !== 0) process.exit(result.status || 1);
cpSync("LICENSE", `${out}/LICENSE.txt`);
const lock = JSON.parse(readFileSync("package-lock.json", "utf8"));
const notices = [
  "Zimsmith — third-party notices",
  readFileSync("crates/engine/assets/fonts/LICENSE.txt", "utf8"),
];
for (const [path, metadata] of Object.entries(lock.packages)) {
  if (!path.startsWith("node_modules/") || metadata.dev) continue;
  for (const file of readdirSync(path).filter((name) =>
    /^(?:licen[cs]e|copying|ofl)(?:[._-].*)?$/i.test(name),
  )) {
    try {
      notices.push(
        `${path} (${metadata.version}) — ${file}\n${readFileSync(`${path}/${file}`, "utf8")}`,
      );
    } catch {
      /* Some packages use a license directory. */
    }
  }
}
writeFileSync(
  `${out}/THIRD-PARTY-NOTICES.txt`,
  notices.join("\n\n----------------\n\n"),
);
if (extension) {
  const manifest = JSON.parse(readFileSync("extension/manifest.json", "utf8"));
  manifest.version = JSON.parse(readFileSync("package.json", "utf8")).version;
  writeFileSync(
    `${out}/manifest.json`,
    JSON.stringify(manifest, null, 2) + "\n",
  );
  cpSync("extension/background.js", `${out}/background.js`);
  mkdirSync(`${out}/icons`, { recursive: true });
  for (const n of [32, 48, 128])
    cpSync(`assets/branding/sizes/${n}x${n}.png`, `${out}/icons/${n}.png`);
  writeFileSync(
    `${out}/INSTALL.txt`,
    "Zimsmith — local browser extension\n\nChrome/Edge: open chrome://extensions or edge://extensions, enable Developer mode, choose Load unpacked, and select this folder. Click the Zimsmith toolbar icon to open the workspace. Grant site access when starting your first crawl. Keep the workspace tab open while jobs run. Closing it preserves checkpoints; reopen and Resume. No server or native helper is used.\n\nThis build supports HTML websites and direct media URLs. YouTube extraction and native Xapian indexing are not included in the browser engine yet. The browser ZIM writer uses uncompressed clusters; archives can be larger than native builds.\n",
  );
} else {
  writeFileSync(
    `${out}/INSTALL.txt`,
    "Zimsmith — browser-only web build\n\nServe these static files over HTTPS or localhost. Crawling, checkpoints, and ZIM building run in the browser; there is no engine server or API to deploy. Source websites must permit CORS. Use the extension package to crawl sites without CORS permission. Keep the workspace tab open while jobs run; export completed ZIMs before clearing browser data.\n\nSupports HTML websites and direct media files. YouTube extraction is not ported yet. ZIM clusters are uncompressed. See the repository README for limits and checks.\n",
  );
}
const zip = resolve(
  `output/builds/Zimsmith-${JSON.parse(readFileSync("package.json", "utf8")).version}-${extension ? "chromium-extension" : "web"}.zip`,
);
mkdirSync(resolve("output/builds"), { recursive: true });
rmSync(zip, { force: true });
const archive = spawnSync("zip", ["-qr", zip, "."], {
  cwd: out,
  stdio: "inherit",
});
if (archive.status !== 0) process.exit(archive.status || 1);
console.log(`Build: ${out}\nZIP: ${zip}`);
