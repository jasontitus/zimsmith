import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { join, basename } from "node:path";
import { tmpdir } from "node:os";
import { createHash } from "node:crypto";
import { prepareRelease } from "./prepare-android-release.mjs";

test(
  "release packager verifies real APK metadata, preserves bytes, escapes notes and requires explicit development-key opt-in",
  { skip: !process.env.ZIMSMITH_TEST_APK },
  () => {
    const dir = mkdtempSync(join(tmpdir(), "zimsmith-release-"));
    try {
      const apk = process.env.ZIMSMITH_TEST_APK;
      const notesFile = join(dir, "notes.txt");
      writeFileSync(
        notesFile,
        '<script>alert("unsafe")</script>\nImproved downloads.',
      );
      const options = {
        apk,
        baseUrl: "https://releases.example",
        output: join(dir, "hosting"),
        notesFile,
      };
      assert.throws(() => prepareRelease(options), /development-signed/);
      const result = prepareRelease({ ...options, allowDevelopmentKey: true });
      const feed = JSON.parse(
        readFileSync(join(result.output, "updates/android.json"), "utf8"),
      );
      assert.equal(
        result.feedUrl,
        "https://releases.example/updates/android.json",
      );
      assert.equal(feed.package_id, "org.zimstudio.app");
      assert.equal(feed.channel, "direct");
      assert.equal(feed.releases.length, 1);
      const source = readFileSync(apk);
      const published = readFileSync(
        join(
          result.output,
          "downloads",
          basename(new URL(result.release.url).pathname),
        ),
      );
      assert.deepEqual(published, source);
      assert.equal(
        result.release.sha256,
        createHash("sha256").update(source).digest("hex"),
      );
      assert.equal(result.release.size_bytes, source.length);
      assert.equal(result.release.certificate_sha256.length, 64);
      const html = readFileSync(join(result.output, "index.html"), "utf8");
      assert.ok(html.includes("&lt;script&gt;"));
      assert.ok(!html.includes("<script>"));
      assert.ok(html.includes("Development build"));
      const github = prepareRelease({
        ...options,
        allowDevelopmentKey: true,
        baseUrl: "https://publisher.github.io/zimsmith/",
        downloadBaseUrl:
          "https://github.com/publisher/zimsmith/releases/download/v0.1.2/",
        sourceUrl: "https://github.com/publisher/zimsmith",
      });
      assert.equal(
        github.feedUrl,
        "https://publisher.github.io/zimsmith/updates/android.json",
      );
      assert.ok(
        github.release.url.startsWith(
          "https://github.com/publisher/zimsmith/releases/download/v0.1.2/",
        ),
      );
      assert.ok(
        readFileSync(join(github.output, "index.html"), "utf8").includes(
          'href="https://github.com/publisher/zimsmith"',
        ),
      );
      prepareRelease({ ...options, allowDevelopmentKey: true });
      assert.equal(
        JSON.parse(readFileSync(join(result.output, "updates/android.json")))
          .releases.length,
        1,
      );
      for (const baseUrl of [
        "http://host",
        "https://user:pass@host/",
        "https://host/path",
        "https://host/?q=1",
      ])
        assert.throws(
          () => prepareRelease({ ...options, baseUrl }),
          /HTTPS origin/,
        );
      const config = JSON.parse(readFileSync("firebase.json", "utf8"));
      assert.equal(config.hosting.public, "output/hosting");
      assert.ok(
        config.hosting.headers
          .find((h) => h.source === "/updates/**")
          .headers.some(
            (h) => h.key === "Cache-Control" && h.value === "no-store",
          ),
      );
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  },
);
