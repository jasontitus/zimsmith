import { test, expect, webkit } from "@playwright/test";
import { createServer } from "node:http";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

// Exercise the real Rust combiner and exported HTML in WebKit, the browser
// engine used by kiwix-apple. The four-second fixtures are generated flash/beeps.
test("gallery closes without navigation and plays the next video on one native WebKit timeline", async () => {
  test.setTimeout(120_000);
  const directory = await mkdtemp(join(tmpdir(), "zim-sync-webkit-"));
  let browser;
  const files = new Map<string, Buffer>();
  const server = createServer((request, response) => {
    const path = new URL(request.url || "/", "http://fixture").pathname.slice(
      1,
    );
    const file = files.get(path);
    if (!file) {
      response.writeHead(404).end();
      return;
    }
    const type = path.endsWith(".html")
      ? "text/html"
      : path.endsWith(".css")
        ? "text/css"
        : path.endsWith(".woff2")
          ? "font/woff2"
          : path.endsWith(".js")
            ? "text/javascript"
            : "video/mp4";
    response.setHeader("Content-Type", type);
    response.setHeader("Accept-Ranges", "bytes");
    const range = /^bytes=(\d+)-(\d*)$/.exec(request.headers.range || "");
    let start = 0,
      end = file.length - 1;
    if (range) {
      start = Number(range[1]);
      end = range[2] ? Math.min(Number(range[2]), end) : end;
      if (start > end) {
        response
          .writeHead(416, { "Content-Range": `bytes */${file.length}` })
          .end();
        return;
      }
      response.statusCode = 206;
      response.setHeader(
        "Content-Range",
        `bytes ${start}-${end}/${file.length}`,
      );
    }
    response.setHeader("Content-Length", end - start + 1);
    response.end(
      request.method === "HEAD" ? undefined : file.subarray(start, end + 1),
    );
  });
  try {
    await promisify(execFile)(
      "cargo",
      [
        "test",
        "-p",
        "zim-engine",
        "preserves_fragmented_av1_bframes_aac_priming_and_every_sample_timestamp",
      ],
      {
        env: { ...process.env, ZIMSTUDIO_SYNC_OUTPUT_DIR: directory },
        timeout: 90_000,
      },
    );
    for (const file of await readdir(directory))
      files.set(file, await readFile(join(directory, file)));
    await new Promise<void>((resolve) =>
      server.listen(0, "127.0.0.1", resolve),
    );
    const address = server.address();
    if (!address || typeof address === "string")
      throw new Error("No fixture port");
    browser = await webkit.launch();
    const page = await browser.newPage({
      viewport: { width: 390, height: 844 },
      isMobile: true,
      hasTouch: true,
    });
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    const prompts: string[] = [];
    page.on("dialog", async (prompt) => {
      prompts.push(prompt.message());
      await prompt.dismiss();
    });
    for (const file of [
      "sync-bframes-video.mp4",
      "sync-fragmented-video.mp4",
    ]) {
      if (file.includes("fragmented")) {
        const av1 = await page.evaluate(() =>
          document
            .createElement("video")
            .canPlayType('video/mp4; codecs="av01.0.04M.08"'),
        );
        if (!av1) {
          test.info().annotations.push({
            type: "info",
            description:
              "Host WebKit has no AV1 decoder; sample/timestamp preservation is covered by Rust tests.",
          });
          continue;
        }
      }
      await page.goto(`http://127.0.0.1:${address.port}/${file}.index.html`);
      const galleryURL = page.url();
      const playButton = page.getByRole("button", {
        name: "Play Sync test: flash and beep",
        exact: true,
      });
      expect(
        await playButton.evaluate((button) => button.closest("a")),
      ).toBeNull();
      expect(await playButton.getAttribute("href")).toBeNull();
      expect(
        await playButton.evaluate((button) =>
          button.dispatchEvent(
            new MouseEvent("contextmenu", { bubbles: true, cancelable: true }),
          ),
        ),
      ).toBe(false);
      await expect(page.locator("video")).toHaveCount(0);
      await page
        .getByRole("button", {
          name: "Play Sync test: flash and beep",
          exact: true,
        })
        .click();
      await expect(page.getByRole("dialog")).toBeVisible();
      await expect(page.locator("video")).toHaveCount(1);
      await expect(page.locator("audio, script[src*='player.js']")).toHaveCount(
        0,
      );
      await expect
        .poll(() =>
          page.locator("video").evaluate((v: HTMLVideoElement) => v.readyState),
        )
        .toBeGreaterThanOrEqual(1);
      await expect
        .poll(() =>
          page
            .locator("video")
            .evaluate((v: HTMLVideoElement) => v.currentTime),
        )
        .toBeGreaterThan(0.4);
      const decoded = await page
        .locator("video")
        .evaluate((v: HTMLVideoElement) => ({
          width: v.videoWidth,
          height: v.videoHeight,
          frames: v.getVideoPlaybackQuality().totalVideoFrames,
          audioTracks: (
            v as HTMLVideoElement & { audioTracks: { length: number } }
          ).audioTracks.length,
          videoTracks: (
            v as HTMLVideoElement & { videoTracks: { length: number } }
          ).videoTracks.length,
        }));
      expect(decoded.audioTracks).toBe(1);
      expect(decoded.videoTracks).toBe(1);
      expect(decoded.width).toBe(160);
      expect(decoded.frames).toBeGreaterThan(0);
      if (file.includes("bframes"))
        await page.screenshot({
          path: test.info().outputPath("player-mobile.png"),
        });
      await page.locator("video").evaluate((v: HTMLVideoElement) => v.pause());
      const paused = await page
        .locator("video")
        .evaluate((v: HTMLVideoElement) => v.currentTime);
      await page.waitForTimeout(250);
      expect(
        await page
          .locator("video")
          .evaluate((v: HTMLVideoElement) => v.currentTime),
      ).toBeCloseTo(paused, 1);
      await page.locator("video").evaluate((v: HTMLVideoElement) => {
        v.currentTime = 2;
      });
      await expect
        .poll(() =>
          page.locator("video").evaluate((v: HTMLVideoElement) => v.seeking),
        )
        .toBe(false);
      expect(
        await page
          .locator("video")
          .evaluate((v: HTMLVideoElement) => v.currentTime),
      ).toBeCloseTo(2, 1);
      await page.locator("video").evaluate((v: HTMLVideoElement) => {
        v.playbackRate = 1.5;
      });
      await page.locator("video").evaluate((v: HTMLVideoElement) => v.play());
      await expect
        .poll(() =>
          page
            .locator("video")
            .evaluate((v: HTMLVideoElement) => v.currentTime),
        )
        .toBeGreaterThan(2.5);
      expect(
        await page
          .locator("video")
          .evaluate((v: HTMLVideoElement) => v.error?.message || null),
      ).toBeNull();
      const player = page.getByRole("dialog");
      await expect(player.locator(".gallery-player-position")).toHaveText(
        "Video 1 of 3",
      );
      await page.locator("video").evaluate((v: HTMLVideoElement) => {
        v.currentTime = v.duration - 0.1;
      });
      await expect
        .poll(() =>
          page.locator("video").evaluate((v: HTMLVideoElement) => v.ended),
        )
        .toBe(true);
      await expect(player).toBeVisible();
      await expect(player.getByRole("status")).toContainText("Finished");
      expect(page.url()).toBe(galleryURL);
      // Ending a clip never opens its page or automatically starts another.
      await expect(player.locator("#gallery-player-title")).toHaveText(
        "Sync test: flash and beep",
      );
      for (const [prefix, label, count] of [
        ["second", "Second video", 2],
        ["third", "Sync finale", 3],
      ] as const) {
        await player
          .getByRole("button", { name: "Next video", exact: true })
          .click();
        await expect(page.locator("video")).toHaveCount(1);
        await expect(player.locator("#gallery-player-title")).toHaveText(label);
        await expect(player.locator(".gallery-player-position")).toHaveText(
          `Video ${count} of 3`,
        );
        await expect
          .poll(() =>
            page
              .locator("video")
              .evaluate((v: HTMLVideoElement) => v.currentSrc),
          )
          .toContain(`/${prefix}-${file}`);
        await expect
          .poll(() =>
            page
              .locator("video")
              .evaluate((v: HTMLVideoElement) => v.currentTime),
          )
          .toBeGreaterThan(0.1);
      }
      await expect(
        player.getByRole("button", { name: "Next video", exact: true }),
      ).toBeDisabled();
      await expect(
        player.getByRole("button", { name: "Close player", exact: true }),
      ).toHaveText("Back to gallery");
      await page
        .getByRole("button", { name: "Close player", exact: true })
        .click();
      await expect(page.getByRole("dialog")).not.toBeVisible();
      await expect(page.locator("video")).toHaveCount(0);
      await expect(
        page.getByRole("button", { name: "Play Sync finale", exact: true }),
      ).toBeFocused();
      expect(page.url()).toBe(galleryURL);
      await page.getByRole("searchbox").fill("Sync");
      await expect(page.locator("[data-document]:visible")).toHaveCount(2);
      await playButton.click();
      await expect(player.locator(".gallery-player-position")).toHaveText(
        "Video 1 of 2",
      );
      await player
        .getByRole("button", { name: "Next video", exact: true })
        .click();
      await expect(player.locator("#gallery-player-title")).toHaveText(
        "Sync finale",
      );
      await expect(
        player.getByRole("button", { name: "Next video", exact: true }),
      ).toBeDisabled();
      await player
        .getByRole("button", { name: "Close player", exact: true })
        .click();
      await expect(page.getByRole("searchbox")).toHaveValue("Sync");
      expect(page.url()).toBe(galleryURL);
      expect(prompts).toEqual([]);
      await page.locator(".entry-detail").first().click();
      await expect(page).toHaveURL(
        new RegExp(`${file.replaceAll(".", "\\.")}\\.html$`),
      );
      await expect(
        page.getByRole("heading", { name: "About this item", exact: true }),
      ).toBeVisible();
      expect(
        await page.locator("video").evaluate((v: HTMLVideoElement) => v.paused),
      ).toBe(true);
      expect(errors).toEqual([]);
    }
  } finally {
    await browser?.close();
    server.close();
    await rm(directory, { recursive: true, force: true });
  }
});
