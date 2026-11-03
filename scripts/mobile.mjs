// Build and check mobile targets with the same Rust toolchain as rustup's std.
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";
import { resolve, join } from "node:path";
import os from "node:os";
import { syncIcons } from "./sync-icons.mjs";
import { syncIOSSharing } from "./sync-ios-sharing.mjs";

const [platform, command = "dev", ...args] = process.argv.slice(2);
if (!["ios", "android"].includes(platform))
  throw new Error(
    "Usage: node scripts/mobile.mjs ios|android dev|build|check [args]",
  );
if (["build", "dev"].includes(command)) syncIcons(platform);
if (platform === "ios" && ["build", "dev"].includes(command))
  syncIOSSharing(resolve("."));
const env = { ...process.env, LZMA_API_STATIC: "1" };
// Release binaries must not embed the developer's home directory in panic
// locations or native-library __FILE__ strings.
if (command === "build") {
  const home = os.homedir();
  env.RUSTFLAGS =
    `${env.RUSTFLAGS || ""} --remap-path-prefix=${home}=/build-user`.trim();
  for (const key of ["CFLAGS", "CXXFLAGS"])
    env[key] = `${env[key] || ""} -ffile-prefix-map=${home}=/build-user`.trim();
}

const rustc = spawnSync("rustup", ["which", "rustc"], { encoding: "utf8" });
if (rustc.status !== 0)
  throw new Error("Install Rust with rustup before building mobile targets");
env.RUSTC = rustc.stdout.trim();

if (platform === "android") {
  const sdk =
    env.ANDROID_HOME ||
    env.ANDROID_SDK_ROOT ||
    join(
      os.homedir(),
      os.platform() === "darwin" ? "Library/Android/sdk" : "Android/Sdk",
    );
  const ndkBase = join(sdk, "ndk");
  const latest = existsSync(ndkBase)
    ? readdirSync(ndkBase)
        .sort((a, b) => a.localeCompare(b, undefined, { numeric: true }))
        .at(-1)
    : undefined;
  const ndk =
    env.NDK_HOME || env.ANDROID_NDK_HOME || (latest && join(ndkBase, latest));
  if (!ndk) throw new Error("Install Android NDK 28+ or set NDK_HOME");
  env.ANDROID_HOME = sdk;
  env.NDK_HOME = ndk;
  env.ANDROID_NDK_HOME = ndk;
  if (os.platform() === "darwin" && !env.JAVA_HOME) {
    const java =
      "/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home";
    if (existsSync(java)) env.JAVA_HOME = java;
  }
  const host =
    os.platform() === "darwin"
      ? "darwin-x86_64"
      : os.platform() === "win32"
        ? "windows-x86_64"
        : "linux-x86_64";
  const toolchain = join(ndk, "toolchains/llvm/prebuilt", host);
  const sysroot = join(toolchain, "sysroot");
  env.BINDGEN_EXTRA_CLANG_ARGS = `--sysroot="${sysroot}" -I"${join(sysroot, "usr/include/aarch64-linux-android")}" -D__ANDROID_API__=24`;
  const extension = os.platform() === "win32" ? ".cmd" : "";
  env.CC_aarch64_linux_android = join(
    toolchain,
    `bin/aarch64-linux-android24-clang${extension}`,
  );
  env.AR_aarch64_linux_android = join(
    toolchain,
    `bin/llvm-ar${os.platform() === "win32" ? ".exe" : ""}`,
  );
}
let result;
if (command === "check") {
  const target =
    platform === "ios" ? "aarch64-apple-ios" : "aarch64-linux-android";
  result = spawnSync(
    "cargo",
    ["check", "-p", "zim-engine", "--target", target, ...args],
    { env, stdio: "inherit" },
  );
} else {
  const cli = resolve("node_modules/@tauri-apps/cli/tauri.js");
  result = spawnSync(
    process.execPath,
    [
      cli,
      platform,
      command,
      ...(platform === "android" && command === "build"
        ? ["--target", "aarch64"]
        : []),
      ...args,
    ],
    { env, stdio: "inherit" },
  );
}
if (result.status === 0 && platform === "ios" && command === "init")
  syncIOSSharing(resolve("."));
process.exit(result.status || 0);
