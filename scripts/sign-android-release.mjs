// Private key material lives outside the repository and is never uploaded.
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, mkdtempSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import os from "node:os";
const [input, output] = process.argv.slice(2);
if (!input || !output)
  throw new Error(
    "Usage: node scripts/sign-android-release.mjs unsigned.apk signed.apk",
  );
const sdk =
  process.env.ANDROID_HOME ||
  process.env.ANDROID_SDK_ROOT ||
  join(
    os.homedir(),
    os.platform() === "darwin" ? "Library/Android/sdk" : "Android/Sdk",
  );
const version = readdirSync(join(sdk, "build-tools"))
  .sort((a, b) => a.localeCompare(b, undefined, { numeric: true }))
  .at(-1);
const tools = join(sdk, "build-tools", version);
const signing =
  process.env.ZIMSMITH_SIGNING_DIR ||
  join(os.homedir(), ".config/zimsmith/signing");
const key = join(signing, "android-release.p12");
const password = join(signing, "android-release.password");
if (!existsSync(key) || !existsSync(password))
  throw new Error(
    "Release signing key/password not found. Set ZIMSMITH_SIGNING_DIR to your private signing directory.",
  );
const env = { ...process.env };
const java = "/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home";
if (!env.JAVA_HOME && existsSync(java)) env.JAVA_HOME = java;
const work = mkdtempSync(join(os.tmpdir(), "zimsmith-sign-"));
function run(name, args) {
  const extension =
    os.platform() === "win32" ? (name === "apksigner" ? ".bat" : ".exe") : "";
  const result = spawnSync(join(tools, name + extension), args, {
    env,
    stdio: "inherit",
  });
  if (result.error || result.status !== 0) throw new Error(`${name} failed`);
}
try {
  const aligned = join(work, "aligned.apk");
  run("zipalign", ["-f", "-P", "16", "4", resolve(input), aligned]);
  run("apksigner", [
    "sign",
    "--ks",
    key,
    "--ks-key-alias",
    "zimsmith",
    "--ks-pass",
    `file:${password}`,
    "--v4-signing-enabled",
    "false",
    "--out",
    resolve(output),
    aligned,
  ]);
  run("apksigner", ["verify", "--print-certs", resolve(output)]);
  run("zipalign", ["-c", "-P", "16", "4", resolve(output)]);
} finally {
  rmSync(work, { recursive: true, force: true });
}
