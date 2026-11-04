import { test, expect } from "@playwright/test";
import { createServer } from "node:http";
import { mkdtemp, readdir, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { extname, join } from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

test("a website's saved YouTube embed plays and seeks locally with captions", async ({
  page,
}) => {
  test.setTimeout(120000);
  const temp = await mkdtemp(join(tmpdir(), "zimsmith-embedded-playback-"));
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
          ".css": "text/css",
          ".mp4": "video/mp4",
          ".vtt": "text/vtt",
          ".woff2": "font/woff2",
        } as Record<string, string>
      )[extname(path)] || "application/octet-stream",
    );
    res.setHeader("Accept-Ranges", "bytes");
    const range = req.headers.range?.match(/^bytes=(\d+)-(\d*)$/);
    if (range) {
      const start = Number(range[1]);
      const end = Math.min(
        range[2] ? Number(range[2]) : file.length - 1,
        file.length - 1,
      );
      if (start > end) {
        res.writeHead(416, { "Content-Range": `bytes */${file.length}` }).end();
        return;
      }
      res.writeHead(206, {
        "Content-Range": `bytes ${start}-${end}/${file.length}`,
        "Content-Length": end - start + 1,
      });
      res.end(file.subarray(start, end + 1));
    } else {
      res.setHeader("Content-Length", file.length);
      res.end(file);
    }
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
        "cached_embedded_video_is_packaged",
        "--locked",
      ],
      {
        env: { ...process.env, ZIMSTUDIO_EMBED_OUTPUT_DIR: temp },
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
    const external: string[] = [];
    page.on("request", (req) => {
      if (!req.url().startsWith(origin)) external.push(req.url());
    });
    await page.goto(`${origin}/home.html`);
    const video = page.locator("video").first();
    await expect(page.locator("video")).toHaveCount(2);
    await expect(video.locator("track")).toHaveAttribute("src", "captions.vtt");
    await video.evaluate(async (el) => {
      const v = el as HTMLVideoElement;
      v.muted = true;
      await v.play();
    });
    await expect
      .poll(() => video.evaluate((el) => (el as HTMLVideoElement).currentTime))
      .toBeGreaterThan(0.25);
    await video.evaluate((el) => {
      const v = el as HTMLVideoElement;
      v.pause();
      v.currentTime = 2;
    });
    await expect
      .poll(() => video.evaluate((el) => (el as HTMLVideoElement).currentTime))
      .toBeCloseTo(2, 1);
    expect(
      await video.evaluate(
        (el) => (el as HTMLVideoElement).error?.message || null,
      ),
    ).toBeNull();
    expect(external).toEqual([]);
    await page
      .getByRole("link", { name: "Offline talk", exact: true })
      .first()
      .click();
    await expect(page).toHaveURL(`${origin}/embedded-video.html`);
  } finally {
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
    await rm(temp, { recursive: true, force: true });
  }
});
