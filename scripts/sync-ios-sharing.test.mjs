import test from "node:test";
import assert from "node:assert/strict";
import YAML from "yaml";
import { configureSharing } from "./sync-ios-sharing.mjs";

test("regenerating a legacy project preserves its supported iOS version and embeds one share extension", () => {
  const project = YAML.parse(
    `options:\n  deploymentTarget:\n    iOS: 15.0\ntargets:\n  zim-studio_iOS:\n    sources: []\n    info:\n      properties: {}\n`,
  );
  const config = {
    identifier: "org.example.app",
    productName: "Example",
    version: "1.2.3",
    bundle: { iOS: { bundleVersion: "7", minimumSystemVersion: "15.0" } },
  };
  const first = configureSharing(project, config);
  const roundTrip = configureSharing(YAML.parse(YAML.stringify(first)), config);
  assert.deepEqual(roundTrip, first);
  assert.equal(roundTrip.targets["zim-studio_iOS"].deploymentTarget, "15.0");
  assert.equal(roundTrip.targets.ZimsmithShare.deploymentTarget, "15.0");
  assert.equal(typeof roundTrip.options.deploymentTarget.iOS, "string");
  assert.equal(
    roundTrip.targets["zim-studio_iOS"].dependencies.filter(
      (d) => d.target === "ZimsmithShare",
    ).length,
    1,
  );
});
