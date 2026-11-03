import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";

const directory = resolve(".tools/yt-dlp");
mkdirSync(resolve(".tools"), { recursive: true });
const args = existsSync(resolve(directory, ".git"))
  ? ["-C", directory, "pull", "--ff-only"]
  : ["clone", "https://github.com/yt-dlp/yt-dlp.git", directory];
const result = spawnSync("git", args, { stdio: "inherit" });
if (result.status !== 0) process.exit(result.status || 1);
spawnSync("git", ["-C", directory, "rev-parse", "HEAD"], { stdio: "inherit" });
