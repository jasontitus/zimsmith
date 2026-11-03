import { chromium, expect, test, type Page } from "@playwright/test";
import { createServer } from "node:http";
import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { extname, join, resolve } from "node:path";
import { execFileSync } from "node:child_process";
import type { Job } from "../src/types";

test.skip(
  !existsSync("output/extension/manifest.json"),
  "Run npm run extension:build first.",
);

async function jobs(page: Page): Promise<Job[]> {
  return page.evaluate(
    () =>
      new Promise((resolve, reject) => {
        const open = indexedDB.open("zimsmith-browser-v1");
        open.onsuccess = () => {
          const request = open.result
            .transaction("jobs")
            .objectStore("jobs")
            .getAll();
          request.onsuccess = () => {
            resolve(request.result);
            open.result.close();
          };
          request.onerror = () => reject(request.error);
        };
        open.onerror = () => reject(open.error);
      }),
  );
}
async function extension() {
  const temp = await mkdtemp(join(tmpdir(), "zimsmith-extension-test-"));
  const dir = join(temp, "extension");
  await cp("output/extension", dir, { recursive: true });
  const manifest = JSON.parse(
    await readFile(join(dir, "manifest.json"), "utf8"),
  );
  // Only this isolated test copy pre-grants hosts, avoiding an interactive
  // Chrome permission dialog. The shipped manifest uses optional permissions.
  manifest.host_permissions = manifest.optional_host_permissions;
  await writeFile(join(dir, "manifest.json"), JSON.stringify(manifest));
  const context = await chromium.launchPersistentContext(
    join(temp, "profile"),
    {
      channel: "chromium",
      headless: true,
      args: [`--disable-extensions-except=${dir}`, `--load-extension=${dir}`],
    },
  );
  context.setDefaultTimeout(10000);
  const sw =
    context.serviceWorkers()[0] ||
    (await context.waitForEvent("serviceworker"));
  const url = `chrome-extension://${sw.url().split("/")[2]}/index.html`;
  const page = await context.newPage();
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(url);
  await expect(page.locator(".engine-status")).toContainText(
    "On-device engine",
  );
  await expect(
    page.getByRole("button", { name: "Scan source", exact: true }),
  ).toBeEnabled({ timeout: 15000 });
  return {
    context,
    page,
    url,
    errors,
    close: async () => {
      await context.close();
      await rm(temp, { recursive: true, force: true });
    },
  };
}
function validateZim(path: string, media?: string) {
  const result = execFileSync(
    "cargo",
    [
      "run",
      "-p",
      "zim-engine",
      "--example",
      "check_browser_zim",
      "--offline",
      "--",
      resolve(path),
      ...(media ? [resolve(media)] : []),
    ],
    { encoding: "utf8", timeout: 120000 },
  );
  expect(result).toContain("Verified browser ZIM");
}

test("extension crawls without CORS, pauses through 429/reload, and exports/rebuilds a real ZIM without a server", async () => {
  test.setTimeout(180000);
  const hits: { path: string; time: number }[] = [];
  let denied = false;
  const server = createServer((request, response) => {
    const path = request.url!;
    hits.push({ path, time: Date.now() });
    // Intentionally no CORS headers: this must use extension privileges.
    if (path === "/robots.txt") {
      response.end(
        "User-agent: *\nDisallow: /source/private\nCrawl-delay: 1\n",
      );
      return;
    }
    if (path === "/source/" || path === "/source/other") {
      response.setHeader("Content-Type", "text/html");
      response.end(
        `<html><head><title>${path.endsWith("other") ? "Molluscs and tide pools" : "Field guide"}</title><link rel="stylesheet" href="/main.css"></head><body onload="alert('bad')"><h1>Offline molluscs</h1><script>fetch('https://never.example/track')</script><a href="/source/other">Other</a><a href="/source/private">Private</a><a href="/outside">Outside scope</a><a href="javascript:alert(1)">Bad</a><img src="/art.svg"></body></html>`,
      );
      return;
    }
    if (path === "/main.css") {
      response.setHeader("Content-Type", "text/css");
      response.end(
        '@import "/nested.css"; body { background-image:url("/art.svg"); }',
      );
      return;
    }
    if (path === "/source/redirect") {
      response.writeHead(302, { Location: "/source/private" });
      response.end();
      return;
    }
    if (path === "/nested.css") {
      response.setHeader("Content-Type", "text/css");
      response.end("h1 { color: purple; }");
      return;
    }
    if (path === "/art.svg") {
      if (!denied) {
        denied = true;
        response.writeHead(429, { "Retry-After": "5" });
        response.end();
        return;
      }
      response.setHeader("Content-Type", "image/svg+xml");
      response.end(
        '<svg xmlns="http://www.w3.org/2000/svg"><circle r="10"/></svg>',
      );
      return;
    }
    response.writeHead(404);
    response.end();
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = (server.address() as { port: number }).port;
  const app = await extension();
  try {
    const { page } = app;
    const outgoing: string[] = [];
    app.context.on("request", (request) => {
      if (/^https?:/.test(request.url())) outgoing.push(request.url());
    });
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page
      .getByLabel("Automatically start small jobs", { exact: true })
      .uncheck();
    await page
      .getByRole("button", { name: "Save download preferences", exact: true })
      .click();
    await expect(page.getByRole("status")).toContainText(
      "Download preferences saved",
    );
    await page
      .getByRole("button", { name: "Create an archive", exact: true })
      .click();
    await page
      .getByLabel("Website URL", { exact: true })
      .fill(`http://127.0.0.1:${port}/source/`);
    await page
      .getByLabel("Archive title", { exact: true })
      .fill("Browser field guide");
    await page
      .getByRole("button", { name: "Crawl & archive settings", exact: true })
      .click();
    await page
      .getByLabel("Request spacing (seconds)", { exact: true })
      .fill("1");
    await page
      .getByRole("button", { name: "Scan source", exact: true })
      .click();
    await expect(
      page.getByText("Ready for review", { exact: true }),
    ).toBeVisible({ timeout: 25000 });
    expect(
      hits.filter((h) =>
        ["/art.svg", "/main.css", "/nested.css", "/source/private"].includes(
          h.path,
        ),
      ),
    ).toEqual([]);
    await page
      .getByRole("button", { name: "Review & build", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Download & build ZIM", exact: true })
      .click();
    await expect
      .poll(async () => (await jobs(page))[0]?.retry_at, { timeout: 20000 })
      .toBeTruthy();
    await page.getByRole("button", { name: /^Pause/ }).click();
    await expect.poll(async () => (await jobs(page))[0]?.status).toBe("paused");
    await page.reload();
    await expect(
      page.getByRole("button", { name: "Scan source", exact: true }),
    ).toBeEnabled();
    await page.getByRole("button", { name: "Activity", exact: true }).click();
    await page.getByRole("button", { name: /^Browser field guide/ }).click();
    await page.getByRole("button", { name: /^Resume$/ }).click();
    await expect(
      page.getByText("Your collection is ready to go.", { exact: true }),
    ).toBeVisible({ timeout: 35000 });
    const art = hits.filter((h) => h.path === "/art.svg");
    expect(art).toHaveLength(2);
    expect(art[1].time - art[0].time).toBeGreaterThanOrEqual(5000);
    expect(hits.filter((h) => h.path === "/source/private")).toEqual([]);
    await mkdir("test-results/browser-extension", { recursive: true });
    const event = page.waitForEvent("download");
    await page
      .getByRole("button", { name: "Save ZIM file", exact: true })
      .click();
    await (await event).saveAs("test-results/browser-extension/atlas.zim");
    validateZim("test-results/browser-extension/atlas.zim");
    const before = hits.length;
    await page
      .getByLabel("Archive template", { exact: true })
      .selectOption("screening_room");
    await page
      .getByRole("button", { name: "Rebuild ZIM", exact: true })
      .click();
    await expect
      .poll(
        async () => {
          const job = (await jobs(page))[0];
          return `${job?.archive_revision}:${job?.status}`;
        },
        { timeout: 15000 },
      )
      .toBe("1:completed");
    expect(hits.length).toBe(before);
    const rebuilt = page.waitForEvent("download");
    await page
      .getByRole("button", { name: "Save ZIM file", exact: true })
      .click();
    const download = await rebuilt;
    expect(download.suggestedFilename()).toMatch(/-r1\.zim$/);
    await download.saveAs("test-results/browser-extension/screening-room.zim");
    validateZim("test-results/browser-extension/screening-room.zim");
    await page.screenshot({
      path: "test-results/browser-extension/completed.png",
      fullPage: true,
    });
    await page
      .getByRole("button", { name: "Create an archive", exact: true })
      .click();
    await page
      .getByLabel("Website URL", { exact: true })
      .fill(`http://127.0.0.1:${port}/source/redirect`);
    await page
      .getByLabel("Archive title", { exact: true })
      .fill("Redirect policy");
    await page
      .getByRole("button", { name: "Scan source", exact: true })
      .click();
    await expect(page.getByRole("alert")).toContainText(
      "before following unverified redirects",
      { timeout: 10000 },
    );
    expect(
      hits.filter((h) => ["/source/private", "/outside"].includes(h.path)),
    ).toEqual([]);
    expect(
      outgoing.every(
        (url) => new URL(url).origin === `http://127.0.0.1:${port}`,
      ),
    ).toBeTruthy();
    expect(app.errors).toEqual([]);
  } finally {
    await app.close();
    server.close();
  }
});

test("media resumes by strong ETag/Range after reload and a second tab cannot own the engine", async () => {
  test.setTimeout(120000);
  const fixture = "crates/engine/tests/fixtures/sync-bframes-video.mp4";
  const bytes = await readFile(fixture);
  const ranges: number[] = [];
  const server = createServer((request, response) => {
    if (request.url === "/robots.txt") {
      response.end("User-agent: *\nAllow: /\n");
      return;
    }
    if (request.url !== "/clip.mp4") {
      response.writeHead(404);
      response.end();
      return;
    }
    const start = Number(
      request.headers.range?.match(/^bytes=(\d+)-$/)?.[1] || 0,
    );
    ranges.push(start);
    response.writeHead(start ? 206 : 200, {
      "Content-Type": "video/mp4",
      "Content-Length": bytes.length - start,
      ETag: '"browser-media-v1"',
      "Accept-Ranges": "bytes",
      ...(start
        ? {
            "Content-Range": `bytes ${start}-${bytes.length - 1}/${bytes.length}`,
          }
        : {}),
    });
    let at = start;
    const timer = setInterval(() => {
      const end = Math.min(at + 64, bytes.length);
      response.write(bytes.subarray(at, end));
      at = end;
      if (at === bytes.length) {
        clearInterval(timer);
        response.end();
      }
    }, 60);
    response.on("close", () => clearInterval(timer));
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = (server.address() as { port: number }).port;
  const app = await extension();
  try {
    const { page } = app;
    const second = await app.context.newPage();
    await second.goto(app.url);
    await expect(second.getByRole("alert")).toContainText(
      "already open in another tab",
    );
    await second.close();
    await page
      .getByLabel("Website URL", { exact: true })
      .fill(`http://127.0.0.1:${port}/clip.mp4`);
    await page
      .getByLabel("Archive title", { exact: true })
      .fill("Resumable browser video");
    await page
      .getByRole("button", { name: "Scan source", exact: true })
      .click();
    await expect
      .poll(async () => (await jobs(page))[0]?.progress?.received || 0, {
        timeout: 15000,
      })
      .toBeGreaterThan(256);
    await page.getByRole("button", { name: /^Pause$/ }).click();
    await expect.poll(async () => (await jobs(page))[0]?.status).toBe("paused");
    await page.reload();
    await expect(
      page.getByRole("button", { name: "Scan source", exact: true }),
    ).toBeEnabled();
    await page.getByRole("button", { name: "Activity", exact: true }).click();
    await page
      .getByRole("button", { name: /^Resumable browser video/ })
      .click();
    await page.getByRole("button", { name: /^Resume$/ }).click();
    await expect(
      page.getByText("Your collection is ready to go.", { exact: true }),
    ).toBeVisible({ timeout: 25000 });
    expect(ranges).toHaveLength(2);
    expect(ranges[0]).toBe(0);
    expect(ranges[1]).toBeGreaterThan(0);
    const download = page.waitForEvent("download");
    await page
      .getByRole("button", { name: "Save ZIM file", exact: true })
      .click();
    await mkdir("test-results/browser-extension", { recursive: true });
    await (
      await download
    ).saveAs("test-results/browser-extension/resumed-video.zim");
    validateZim("test-results/browser-extension/resumed-video.zim", fixture);
    expect(app.errors).toEqual([]);
  } finally {
    await app.close();
    server.close();
  }
});

test("static web build crawls CORS-enabled sources locally and clearly rejects a blocked source", async ({
  page,
}) => {
  test.skip(
    !existsSync("output/web/index.html"),
    "Run npm run web:build first.",
  );
  test.setTimeout(90000);
  const application = createServer(async (request, response) => {
    const path = new URL(request.url!, "http://localhost").pathname;
    try {
      const file = resolve(
        "output/web",
        `.${path === "/" ? "/index.html" : path}`,
      );
      if (!file.startsWith(resolve("output/web") + "/"))
        throw new Error("invalid path");
      response.setHeader(
        "Content-Type",
        (
          {
            ".html": "text/html",
            ".js": "application/javascript",
            ".css": "text/css",
            ".woff2": "font/woff2",
            ".png": "image/png",
          } as Record<string, string>
        )[extname(file)] || "application/octet-stream",
      );
      response.end(await readFile(file));
    } catch {
      response.writeHead(404);
      response.end();
    }
  });
  const source = createServer((request, response) => {
    if (request.url !== "/blocked/")
      response.setHeader("Access-Control-Allow-Origin", "*");
    if (request.url === "/robots.txt") {
      response.end("User-agent: *\nAllow: /\n");
      return;
    }
    response.setHeader("Content-Type", "text/html");
    response.end(
      "<html><head><title>Local browser collection</title></head><body><h1>Molluscs to explore offline</h1></body></html>",
    );
  });
  await new Promise<void>((resolve) =>
    application.listen(0, "127.0.0.1", resolve),
  );
  await new Promise<void>((resolve) => source.listen(0, "127.0.0.1", resolve));
  const appPort = (application.address() as { port: number }).port,
    sourcePort = (source.address() as { port: number }).port;
  const requests: string[] = [];
  page.context().on("request", (request) => requests.push(request.url()));
  try {
    await page.goto(`http://127.0.0.1:${appPort}/`);
    await expect(
      page.getByRole("button", { name: "Scan source", exact: true }),
    ).toBeEnabled();
    await page
      .getByLabel("Website URL", { exact: true })
      .fill(`http://127.0.0.1:${sourcePort}/source/`);
    await page
      .getByLabel("Archive title", { exact: true })
      .fill("Standalone browser archive");
    await page
      .getByRole("button", { name: "Scan source", exact: true })
      .click();
    await expect(
      page.getByText("Your collection is ready to go.", { exact: true }),
    ).toBeVisible({ timeout: 20000 });
    const download = page.waitForEvent("download");
    await page
      .getByRole("button", { name: "Save ZIM file", exact: true })
      .click();
    await mkdir("test-results/browser-extension", { recursive: true });
    await (
      await download
    ).saveAs("test-results/browser-extension/standalone.zim");
    validateZim("test-results/browser-extension/standalone.zim");
    await page
      .getByRole("button", { name: "Create an archive", exact: true })
      .click();
    await page
      .getByLabel("Website URL", { exact: true })
      .fill(`http://127.0.0.1:${sourcePort}/blocked/`);
    await page
      .getByLabel("Archive title", { exact: true })
      .fill("CORS restriction");
    await page
      .getByRole("button", { name: "Scan source", exact: true })
      .click();
    await expect(page.getByRole("alert")).toContainText(
      "source must allow CORS",
      { timeout: 30000 },
    );
    const failed = (await jobs(page)).find(
      (job) => job.config.title === "CORS restriction",
    )!;
    await page.getByRole("button", { name: "Edit crawl", exact: true }).click();
    await page
      .getByLabel("Website URL", { exact: true })
      .fill(`http://127.0.0.1:${sourcePort}/source/`);
    await page
      .getByRole("button", { name: "Save & retry", exact: true })
      .click();
    await expect(
      page.getByText("Your collection is ready to go.", { exact: true }),
    ).toBeVisible({ timeout: 20000 });
    expect((await jobs(page)).find((job) => job.id === failed.id)?.status).toBe(
      "completed",
    );
    expect(
      requests.some((url) => url.includes(":8788/") || /\/api\//.test(url)),
    ).toBeFalsy();
  } finally {
    application.close();
    source.close();
  }
});

test("CDN fonts skip the page delay while preserving declared delays and 429 backoff", async () => {
  test.setTimeout(90000);
  const server = createServer((request, response) => {
    if (request.url === "/robots.txt") {
      response.end("User-agent: *\nAllow: /\n");
      return;
    }
    response.setHeader("Content-Type", "text/html");
    response.end(
      '<html><head><title>Fast font assets</title><link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=BrowserTest"></head><body>Fonts saved offline</body></html>',
    );
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = (server.address() as { port: number }).port;
  const app = await extension();
  const fonts = await readFile("crates/engine/assets/fonts/dm-sans.woff2");
  const hits: { path: string; time: number }[] = [];
  let limited = false;
  try {
    await app.context.route(
      /^https:\/\/fonts\.(googleapis|gstatic)\.com\//,
      async (route) => {
        const url = new URL(route.request().url());
        if (url.pathname === "/robots.txt") {
          await route.fulfill({
            contentType: "text/plain",
            body: `User-agent: *\nAllow: /\n${url.hostname === "fonts.gstatic.com" ? "Crawl-delay: 0.1\n" : ""}`,
          });
          return;
        }
        if (url.hostname === "fonts.googleapis.com") {
          await route.fulfill({
            contentType: "text/css",
            body: ["one", "two", "limited"]
              .map(
                (n) =>
                  `@font-face{font-family:font-${n};src:url(https://fonts.gstatic.com/${n}.woff2)}`,
              )
              .join("\n"),
          });
          return;
        }
        hits.push({ path: url.pathname, time: Date.now() });
        if (url.pathname === "/limited.woff2" && !limited) {
          limited = true;
          await route.fulfill({
            status: 429,
            headers: { "Retry-After": "2" },
            body: "wait",
          });
          return;
        }
        await route.fulfill({ contentType: "font/woff2", body: fonts });
      },
    );
    const { page } = app;
    await page
      .getByLabel("Website URL", { exact: true })
      .fill(`http://127.0.0.1:${port}/`);
    await page
      .getByLabel("Archive title", { exact: true })
      .fill("Fast CDN fonts");
    await page
      .getByRole("button", { name: "Crawl & archive settings", exact: true })
      .click();
    await page
      .getByLabel("Request spacing (seconds)", { exact: true })
      .fill("4");
    await page
      .getByRole("button", { name: "Scan source", exact: true })
      .click();
    await expect(
      page.getByText("Your collection is ready to go.", { exact: true }),
    ).toBeVisible({ timeout: 35000 });
    const one = hits.find((h) => h.path === "/one.woff2")!,
      two = hits.find((h) => h.path === "/two.woff2")!;
    expect(Math.abs(two.time - one.time)).toBeGreaterThanOrEqual(80);
    expect(Math.abs(two.time - one.time)).toBeLessThan(2000);
    const retries = hits.filter((h) => h.path === "/limited.woff2");
    expect(retries).toHaveLength(2);
    expect(retries[1].time - retries[0].time).toBeGreaterThanOrEqual(5000);
    const download = page.waitForEvent("download");
    await page
      .getByRole("button", { name: "Save ZIM file", exact: true })
      .click();
    await mkdir("test-results/browser-extension", { recursive: true });
    await (
      await download
    ).saveAs("test-results/browser-extension/cdn-fonts.zim");
    validateZim("test-results/browser-extension/cdn-fonts.zim");
    expect(app.errors).toEqual([]);
  } finally {
    await app.close();
    server.close();
  }
});
