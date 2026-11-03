// Portable static hosting output. No Firebase account or deployment is required.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
  existsSync,
} from "node:fs";
import { join, resolve } from "node:path";
import os from "node:os";
import { pathToFileURL } from "node:url";

const escapeHtml = (s) =>
  s.replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ],
  );
export function prepareRelease(options) {
  const base = new URL(options.baseUrl);
  if (
    base.protocol !== "https:" ||
    base.username ||
    base.password ||
    base.search ||
    base.hash ||
    !base.pathname.endsWith("/")
  )
    throw new Error(
      "base-url must be an HTTPS origin or directory without credentials, query, or fragment; include a trailing slash for a project path",
    );
  const downloads = options.downloadBaseUrl
    ? new URL(options.downloadBaseUrl)
    : new URL("downloads/", base);
  if (
    downloads.protocol !== "https:" ||
    downloads.username ||
    downloads.password ||
    downloads.search ||
    downloads.hash ||
    !downloads.pathname.endsWith("/")
  )
    throw new Error("download-base-url must be an HTTPS directory URL");
  const sourceUrl = options.sourceUrl ? new URL(options.sourceUrl) : null;
  if (
    sourceUrl &&
    (sourceUrl.protocol !== "https:" ||
      sourceUrl.username ||
      sourceUrl.password)
  )
    throw new Error("source-url must use HTTPS without credentials");
  const releasesUrl =
    sourceUrl?.hostname === "github.com"
      ? new URL(
          `${sourceUrl.pathname.split("/").slice(0, 3).join("/")}/releases`,
          sourceUrl.origin,
        ).href
      : null;
  const sdk =
    process.env.ANDROID_HOME ||
    process.env.ANDROID_SDK_ROOT ||
    join(
      os.homedir(),
      os.platform() === "darwin" ? "Library/Android/sdk" : "Android/Sdk",
    );
  const latest = readdirSync(join(sdk, "build-tools"))
    .sort((a, b) => a.localeCompare(b, undefined, { numeric: true }))
    .at(-1);
  const tools = join(sdk, "build-tools", latest);
  const env = { ...process.env };
  const java = "/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home";
  if (!env.JAVA_HOME && existsSync(java)) env.JAVA_HOME = java;
  const run = (name, args) => {
    const result = spawnSync(
      join(
        tools,
        name +
          (os.platform() === "win32"
            ? name === "apksigner"
              ? ".bat"
              : ".exe"
            : ""),
      ),
      args,
      { encoding: "utf8", env, maxBuffer: 4 * 1024 * 1024 },
    );
    if (result.error || result.status !== 0)
      throw new Error(`${name} failed: ${result.error ?? result.stderr}`);
    return result.stdout;
  };
  const apk = resolve(options.apk);
  const badging = run("aapt2", ["dump", "badging", apk]);
  const certificate = run("apksigner", ["verify", "--print-certs", apk]);
  const packageInfo = badging.match(
    /^package: name='([^']+)' versionCode='(\d+)' versionName='([^']+)'/m,
  );
  const minSdk =
    badging.match(/^sdkVersion:'(\d+)'/m) ??
    badging.match(/^minSdkVersion:'(\d+)'/m);
  const abis = [
    ...(badging.match(/^native-code: (.+)$/m)?.[1] ?? "").matchAll(
      /'([^']+)'/g,
    ),
  ].map((m) => m[1]);
  const digests = [
    ...certificate.matchAll(
      /^Signer #\d+ certificate SHA-256 digest: ([0-9a-f]{64})$/gim,
    ),
  ];
  if (
    !packageInfo ||
    packageInfo[1] !== "org.zimstudio.app" ||
    !minSdk ||
    abis.length !== 1 ||
    digests.length !== 1
  )
    throw new Error(
      "Expected a single-ABI Zimsmith APK with one verified signer",
    );
  const development =
    /CN=Android Debug/i.test(certificate) ||
    /^application-debuggable/m.test(badging);
  if (development && !options.allowDevelopmentKey)
    throw new Error(
      "Refusing a development-signed/debuggable APK. Use a production signing key, or explicitly pass --allow-development-key for local testing only.",
    );
  if (
    /REQUEST_INSTALL_PACKAGES|UPDATE_PACKAGES_WITHOUT_USER_ACTION/.test(badging)
  )
    throw new Error(
      "The browser-handoff build must not request app installation permissions",
    );
  const notes = options.notesFile
    ? readFileSync(options.notesFile, "utf8").trim()
    : "";
  if (Buffer.byteLength(notes) > 8000)
    throw new Error("Release notes must fit in 8000 UTF-8 bytes");
  const bytes = readFileSync(apk);
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  const version = packageInfo[3];
  if (!/^[\w.+-]+$/.test(version) || !/^[\w-]+$/.test(abis[0]))
    throw new Error("Unsupported version or ABI filename");
  const filename = `Zimsmith-${version}-${abis[0]}-${sha256.slice(0, 12)}.apk`;
  const output = resolve(options.output ?? "output/hosting");
  const feedPath = join(output, "updates/android.json");
  const release = {
    version,
    version_code: Number(packageInfo[2]),
    abi: abis[0],
    min_sdk: Number(minSdk[1]),
    url: new URL(filename, downloads).href,
    sha256,
    certificate_sha256: digests[0][1].toLowerCase(),
    size_bytes: bytes.length,
    notes,
  };
  if (release.version_code <= 0 || release.version_code > 2147483647)
    throw new Error("Invalid Android version code");
  let releases = [];
  if (existsSync(feedPath)) {
    const previous = JSON.parse(readFileSync(feedPath, "utf8"));
    if (
      previous.schema_version !== 1 ||
      previous.package_id !== packageInfo[1] ||
      previous.channel !== "direct"
    )
      throw new Error("Existing output contains an incompatible release feed");
    releases = previous.releases.filter((r) => r.abi !== release.abi);
  }
  releases.push(release);
  const feed = {
    schema_version: 1,
    package_id: packageInfo[1],
    channel: "direct",
    releases,
  };
  const json = JSON.stringify(feed, null, 2) + "\n";
  if (Buffer.byteLength(json) > 65536)
    throw new Error("Release feed exceeds app limit");
  mkdirSync(join(output, "downloads"), { recursive: true });
  mkdirSync(join(output, "updates"), { recursive: true });
  copyFileSync(apk, join(output, "downloads", filename));
  copyFileSync("assets/branding/sizes/128x128.png", join(output, "icon.png"));
  copyFileSync("LICENSE", join(output, "LICENSE.txt"));
  writeFileSync(feedPath, json);
  writeFileSync(
    join(output, "SHA256SUMS"),
    `${sha256}  downloads/${filename}\n`,
  );
  const warning = development
    ? "<p class=notice>Development build for private testing. Not a public production release.</p>"
    : "";
  writeFileSync(
    join(output, "index.html"),
    `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Zimsmith for Android</title><style>body{font:18px/1.6 system-ui;background:#f6f4fb;color:#211c35;max-width:720px;margin:60px auto;padding:24px}img{width:80px;border-radius:20px}a{color:#5630b8}a.download{display:inline-block;background:#5630b8;color:white;padding:14px 22px;border-radius:12px;text-decoration:none}.notice{background:#fff0cf;padding:16px}pre{white-space:pre-wrap;overflow-wrap:anywhere}small{overflow-wrap:anywhere}footer{margin-top:40px}</style><img src="icon.png" alt=""><h1>Zimsmith for Android</h1><p>Crawl on your device. Build an offline library.</p>${warning}<h2>Version ${escapeHtml(version)}</h2><p>${(bytes.length / 1024 / 1024).toFixed(1)} MB · ${escapeHtml(release.abi)} · Android API ${release.min_sdk}+</p><a class="download" href="${escapeHtml(release.url)}">Download APK</a><p>Open the download, allow your browser to install apps if Android asks, and confirm installation. Future updates keep your jobs when installed over the same app with the same signing key.</p><h2>What’s new</h2><pre>${escapeHtml(notes || "No release notes provided.")}</pre><footer>${sourceUrl ? `<a href="${escapeHtml(sourceUrl.href)}">Source code</a> · ` : ""}${releasesUrl ? `<a href="${escapeHtml(releasesUrl)}">All downloads</a> · ` : ""}<a href="LICENSE.txt">GPL-3.0 license</a> · <a href="updates/android.json">Release feed</a> · <a href="SHA256SUMS">SHA-256 checksum</a><p>You can also track this website in Obtainium using the HTML source. No separate updater app is required for Zimsmith’s built-in update checks.</p><small>APK SHA-256: ${sha256}</small></footer></html>`,
  );
  return {
    output,
    feedUrl: new URL("updates/android.json", base).href,
    release,
    development,
  };
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  const args = process.argv.slice(2);
  const options = {};
  const names = {
    "--apk": "apk",
    "--base-url": "baseUrl",
    "--notes-file": "notesFile",
    "--output": "output",
    "--download-base-url": "downloadBaseUrl",
    "--source-url": "sourceUrl",
  };
  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--allow-development-key")
      options.allowDevelopmentKey = true;
    else if (names[args[i]] && args[i + 1] && !args[i + 1].startsWith("--"))
      options[names[args[i]]] = args[++i];
    else throw new Error(`Unknown or incomplete argument: ${args[i]}`);
  }
  if (!options.apk || !options.baseUrl)
    throw new Error(
      "Usage: npm run release:android -- --apk SIGNED.apk --base-url https://YOUR_HOST [--notes-file notes.txt] [--output output/hosting]",
    );
  console.log(JSON.stringify(prepareRelease(options), null, 2));
}
