import { test, expect } from "@playwright/test";
import { createServer } from "node:http";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, extname } from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

test("caption search opens a playback timestamp and the gallery carries subtitle tracks", async ({
  page,
}) => {
  test.setTimeout(120000);
  const temp = await mkdtemp(join(tmpdir(), "zimsmith-caption-test-"));
  const files = new Map<string, Buffer>();
  const server = createServer((req, res) => {
    const path = new URL(req.url || "/", "http://local").pathname.slice(1);
    const file = files.get(path);
    if (!file) {
      res.writeHead(404).end();
      return;
    }
    res.setHeader(
      "Content-Type",
      (
        {
          ".html": "text/html",
          ".js": "text/javascript",
          ".css": "text/css",
          ".woff2": "font/woff2",
          ".vtt": "text/vtt",
          ".mp4": "video/mp4",
        } as Record<string, string>
      )[extname(path)] || "application/octet-stream",
    );
    res.end(file);
  });
  try {
    await promisify(execFile)(
      "cargo",
      [
        "test",
        "-p",
        "zim-engine",
        "--test",
        "workflow",
        "caption_archive_packages",
        "--locked",
      ],
      {
        env: { ...process.env, ZIMSTUDIO_CAPTIONS_OUTPUT_DIR: temp },
        timeout: 90000,
      },
    );
    for (const name of await readdir(temp))
      files.set(name, await readFile(join(temp, name)));
    await new Promise<void>((resolve) =>
      server.listen(0, "127.0.0.1", resolve),
    );
    const address = server.address();
    if (!address || typeof address === "string")
      throw new Error("No fixture port");
    const origin = `http://127.0.0.1:${address.port}`;
    await page.addInitScript(() => {
      const proto = HTMLMediaElement.prototype;
      Object.defineProperty(proto, "readyState", {
        get: () => 1,
        configurable: true,
      });
      Object.defineProperty(proto, "duration", {
        get: () => 20,
        configurable: true,
      });
      Object.defineProperty(proto, "currentTime", {
        get() {
          return (this as any).fixtureTime || 0;
        },
        set(value) {
          (this as any).fixtureTime = value;
        },
        configurable: true,
      });
      proto.play = function () {
        (this as any).fixturePlay = true;
        return Promise.resolve();
      };
      proto.pause = function () {};
      proto.load = function () {};
    });
    await page.goto(origin + "/index.html");
    await page
      .getByLabel("Search this collection", { exact: true })
      .fill("moonlight");
    const match = page.getByRole("link", { name: /Play at 0:01/ });
    await expect(match).toHaveAttribute("href", "home.html#t=1.25");
    await match.click();
    await expect(page.locator("#video-track")).toHaveJSProperty(
      "currentTime",
      1.25,
    );
    await expect(page.locator("track")).toHaveAttribute("srclang", "en");
    await expect(page.locator(".transcript-cue")).toContainText(
      "<script> & stars",
    );
    await page
      .getByLabel("Search this transcript", { exact: true })
      .fill("not present");
    await expect(page.locator(".transcript-cue")).toBeHidden();
    await page
      .getByLabel("Search this transcript", { exact: true })
      .fill("moonlight");
    await page.locator(".transcript-cue a").click();
    expect(
      await page
        .locator("#video-track")
        .evaluate((el) => (el as any).fixturePlay),
    ).toBe(true);
    await page.goto(origin + "/index.html");
    await page.getByRole("button", { name: "Play Home", exact: true }).click();
    await expect(page.locator(".gallery-player track")).toHaveAttribute(
      "src",
      "captions.vtt",
    );
    await expect(page.locator(".gallery-player track")).toHaveJSProperty(
      "default",
      true,
    );
    await page
      .getByRole("button", { name: "Close player", exact: true })
      .click();
    await expect(page.locator(".gallery-player")).not.toBeVisible();
  } finally {
    const closed = new Promise<void>((resolve) =>
      server.close(() => resolve()),
    );
    // Chromium can leave a preconnected/media socket open after assertions.
    server.closeAllConnections();
    await closed;
    await rm(temp, { recursive: true, force: true });
  }
});
