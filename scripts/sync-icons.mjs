import {
  cpSync,
  existsSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

export function syncIcons(platform) {
  const icons = join(root, "src-tauri/icons");
  if (platform === "ios") {
    const assets = join(
      root,
      "src-tauri/gen/apple/Assets.xcassets/AppIcon.appiconset",
    );
    if (!existsSync(assets)) return;
    const contents = JSON.parse(
      readFileSync(join(assets, "Contents.json"), "utf8"),
    );
    for (const image of contents.images) {
      if (!image.filename) continue;
      const source = join(icons, "ios", image.filename);
      if (!existsSync(source))
        throw new Error(`Missing generated iOS icon: ${image.filename}`);
      cpSync(source, join(assets, image.filename));
    }
  } else if (platform === "android") {
    const resources = join(root, "src-tauri/gen/android/app/src/main/res");
    if (!existsSync(resources)) return;
    for (const directory of readdirSync(join(icons, "android"))) {
      cpSync(join(icons, "android", directory), join(resources, directory), {
        recursive: true,
      });
    }
    const strings = join(resources, "values/strings.xml");
    const { productName } = JSON.parse(
      readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"),
    );
    const name = productName
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;");
    writeFileSync(
      strings,
      readFileSync(strings, "utf8").replace(
        /(<string name="(?:app_name|main_activity_title)">).*?(<\/string>)/g,
        `$1${name}$2`,
      ),
    );
  } else {
    throw new Error(`Unknown mobile platform: ${platform}`);
  }
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  for (const platform of process.argv[2]
    ? [process.argv[2]]
    : ["ios", "android"])
    syncIcons(platform);
}
