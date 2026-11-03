// Store builds deliberately leave the direct-updates Cargo feature disabled.
import { spawnSync } from "node:child_process";
const [command = "build", ...args] = process.argv.slice(2);
if (!["build", "dev"].includes(command))
  throw new Error("Use android:direct -- build|dev [Tauri arguments]");
const result = spawnSync(
  process.execPath,
  [
    "scripts/mobile.mjs",
    "android",
    command,
    ...args,
    "--features",
    "direct-updates",
  ],
  { stdio: "inherit", env: process.env },
);
if (result.error) throw result.error;
process.exit(result.status ?? 1);
