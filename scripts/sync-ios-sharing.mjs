// Keep the extension definition in source control; generated Xcode projects are private.
import { cpSync, existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import YAML from "yaml";

export function configureSharing(project, config) {
  const app = project.targets["zim-studio_iOS"];
  if (!app) throw new Error("The generated iOS application target is missing");
  const version = config.version;
  const build = config.bundle.iOS.bundleVersion;
  const group = `group.${config.identifier}`;
  const minimum = config.bundle.iOS.minimumSystemVersion || "15.0";
  // YAML parsers can turn 15.0 into the number 15; XcodeGen then silently
  // defaults to the newest installed SDK. Set an explicit string per target.
  app.deploymentTarget = minimum;
  project.options ||= {};
  project.options.deploymentTarget ||= {};
  project.options.deploymentTarget.iOS = minimum;
  app.info.properties.CFBundleShortVersionString = version;
  app.info.properties.CFBundleVersion = build;
  // XcodeGen otherwise treats libapp.a copies from past builds as resources.
  const externals = app.sources.find(
    (s) => typeof s === "object" && s.path === "Externals",
  );
  if (externals)
    externals.excludes = [
      ...new Set([...(externals.excludes || []), "**/*.a"]),
    ];
  app.entitlements = {
    ...app.entitlements,
    properties: {
      ...(app.entitlements?.properties || {}),
      "com.apple.security.application-groups": [group],
    },
  };
  app.dependencies = (app.dependencies || []).filter(
    (d) => d.target !== "ZimsmithShare",
  );
  app.dependencies.push({ target: "ZimsmithShare", embed: true });
  project.targets.ZimsmithShare = {
    type: "app-extension",
    platform: "iOS",
    deploymentTarget: minimum,
    sources: [
      { path: "ShareExtension", excludes: ["ZimsmithPage.js"] },
      { path: "ShareExtension/ZimsmithPage.js", buildPhase: "resources" },
    ],
    info: {
      path: "ShareExtension/Info.plist",
      properties: {
        CFBundleDisplayName: config.productName,
        CFBundleShortVersionString: version,
        CFBundleVersion: build,
        NSExtension: {
          NSExtensionPointIdentifier: "com.apple.share-services",
          NSExtensionPrincipalClass:
            "$(PRODUCT_MODULE_NAME).ShareViewController",
          NSExtensionAttributes: {
            NSExtensionJavaScriptPreprocessingFile: "ZimsmithPage",
            NSExtensionActivationRule: {
              NSExtensionActivationDictionaryVersion: 2,
              NSExtensionActivationSupportsWebURLWithMaxCount: 1,
              NSExtensionActivationSupportsWebPageWithMaxCount: 1,
              NSExtensionActivationSupportsText: true,
            },
          },
        },
      },
    },
    entitlements: {
      path: "ShareExtension/Share.entitlements",
      properties: { "com.apple.security.application-groups": [group] },
    },
    settings: {
      base: {
        PRODUCT_BUNDLE_IDENTIFIER: `${config.identifier}.share`,
        PRODUCT_NAME: "ZimsmithShare",
        SWIFT_VERSION: "5.0",
        APPLICATION_EXTENSION_API_ONLY: true,
        SKIP_INSTALL: true,
        CODE_SIGN_STYLE: "Automatic",
        ...(config.bundle.iOS.developmentTeam
          ? { DEVELOPMENT_TEAM: config.bundle.iOS.developmentTeam }
          : {}),
      },
    },
    dependencies: [
      { sdk: "UIKit.framework" },
      { sdk: "UniformTypeIdentifiers.framework" },
    ],
  };
  return project;
}

export function syncIOSSharing(root, configOverride) {
  const directory = join(root, "src-tauri/gen/apple");
  const spec = join(directory, "project.yml");
  if (!existsSync(spec)) return;
  const config = JSON.parse(
    readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"),
  );
  const project = YAML.parse(readFileSync(spec, "utf8"));
  // Keep the locally selected signing team private in the generated project.
  const pbx = join(directory, "zim-studio.xcodeproj/project.pbxproj");
  const team =
    process.env.APPLE_DEVELOPMENT_TEAM ||
    configOverride?.bundle?.iOS?.developmentTeam ||
    (existsSync(pbx) &&
      readFileSync(pbx, "utf8").match(/DEVELOPMENT_TEAM = "?([A-Z0-9]+)/)?.[1]);
  if (team) config.bundle.iOS.developmentTeam = team;
  if (team) {
    project.targets["zim-studio_iOS"].settings.base.DEVELOPMENT_TEAM = team;
    project.targets["zim-studio_iOS"].settings.base.CODE_SIGN_STYLE =
      "Automatic";
  }
  configureSharing(project, config);
  cpSync(
    join(root, "src-tauri/share-extension"),
    join(directory, "ShareExtension"),
    { recursive: true },
  );
  cpSync(
    join(root, "crates/mobile-files/ios/Sources/SharedURLInbox.swift"),
    join(directory, "ShareExtension/SharedURLInbox.swift"),
  );
  writeFileSync(spec, YAML.stringify(project));
  const result = spawnSync("xcodegen", ["generate", "--spec", spec], {
    cwd: directory,
    stdio: "inherit",
  });
  if (result.error || result.status !== 0)
    throw new Error(
      "Could not configure the iOS Share Extension. Install XcodeGen and retry.",
    );
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  syncIOSSharing(resolve(dirname(fileURLToPath(import.meta.url)), ".."));
}
