import { test, expect } from "@playwright/test";
import { createServer } from "node:http";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const api = "http://127.0.0.1:8788/api";
const styles = ["atlas", "field_notes", "midnight", "screening_room"] as const;

test("template selection persists and real exported pages look and search correctly offline", async ({
  page,
  request,
}) => {
  const preferences = await (await request.get(`${api}/settings`)).json();
  const capabilities = await (await request.get(`${api}/health`)).json();
  const files = new Map<string, Buffer>();
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const server = createServer((req, response) => {
    const pathname = new URL(req.url || "/", "http://localhost").pathname;
    const file = files.get(pathname);
    if (file) {
      const ext = pathname.split(".").at(-1)!;
      const mime: Record<string, string> = {
        html: "text/html; charset=utf-8",
        css: "text/css",
        js: "text/javascript",
        woff2: "font/woff2",
        png: "image/png",
        txt: "text/plain",
      };
      response.setHeader(
        "Content-Type",
        mime[ext] || "application/octet-stream",
      );
      response.end(file);
      return;
    }
    if (pathname === "/robots.txt") {
      response.end("User-agent: *\nAllow: /source/\n");
      return;
    }
    if (pathname.startsWith("/source/")) {
      response.setHeader("Content-Type", "text/html; charset=utf-8");
      response.end(
        pathname === "/source/guide"
          ? "<html><head><title>Molluscs of the forest</title></head><body><h1>Molluscs of the forest</h1><p>Discover the plants, patterns, and stories hiding just beyond the path. Molluscs are part of this quiet world.</p></body></html>"
          : "<html><head><title>The art of noticing</title></head><body><h1>The art of noticing</h1><p>Slow down, look closer, and find the extraordinary in the everyday. A field guide to paying attention.</p><a href='/source/guide'>Explore the forest</a></body></html>",
      );
      return;
    }
    response.statusCode = 404;
    response.end("Not found");
  });
  await new Promise<void>((resolve) =>
    server.listen(9767, "127.0.0.1", resolve),
  );
  try {
    await page.goto("/");
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await expect(
      page.getByRole("radiogroup", { name: "Archive index template" }),
    ).toBeVisible();
    await page
      .getByRole("radio", { name: "Screening Room", exact: true })
      .check();
    await expect(
      page
        .frameLocator('iframe[title="Screening Room index page preview"]')
        .locator("body"),
    ).toHaveAttribute("data-zim-template", "screening_room");
    await page
      .getByRole("button", { name: "Save archive style", exact: true })
      .click();
    await expect(page.getByRole("status")).toContainText("Archive style saved");
    await page.reload();
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await expect(
      page.getByRole("radio", { name: "Screening Room", exact: true }),
    ).toBeChecked();
    await expect(
      page
        .frameLocator('iframe[title="Screening Room index page preview"]')
        .locator("body"),
    ).toHaveAttribute("data-zim-template", "screening_room");
    await mkdir("test-results/archive-templates", { recursive: true });
    await page.screenshot({
      path: "test-results/archive-templates/settings-desktop.png",
      fullPage: true,
    });
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth > innerWidth,
      ),
    ).toBe(false);
    await page.screenshot({
      path: "test-results/archive-templates/settings-mobile.png",
      fullPage: true,
    });

    for (const style of styles) {
      await request.post(`${api}/settings`, {
        data: {
          ...preferences,
          auto_start_small_jobs: true,
          auto_start_item_limit: 100,
          archive_template: style,
        },
      });
      const created = await request.post(`${api}/jobs`, {
        data: {
          url: "http://127.0.0.1:9767/source/",
          title: "A world worth exploring.",
          description:
            "Field guides, quiet discoveries, and ideas worth keeping.",
          creator: "The curious reader",
          language: "eng",
          kind: "website",
          max_items: 10,
          max_depth: 1,
          delay_ms: 1000,
          max_resource_mb: 2,
          include_assets: false,
        },
      });
      expect(created.ok()).toBe(true);
      const job = await created.json();
      await expect
        .poll(
          async () => {
            const current = await (
              await request.get(`${api}/jobs/${job.id}`)
            ).json();
            if (current.status === "failed") throw new Error(current.error);
            return current.status;
          },
          { timeout: 30000 },
        )
        .toBe("completed");
      const content = resolve(capabilities.data_dir, job.id, "content");
      for (const name of await readdir(content))
        files.set(
          `/archives/${style}/${name}`,
          await readFile(resolve(content, name)),
        );
      const artifact = await request.get(`${api}/jobs/${job.id}/artifact`);
      expect(artifact.ok()).toBe(true);
      await writeFile(
        `test-results/archive-templates/${style}.zim`,
        await artifact.body(),
      );

      const outside: string[] = [];
      await page.route("**/*", (route) => {
        if (new URL(route.request().url()).origin !== "http://127.0.0.1:9767") {
          outside.push(route.request().url());
          return route.abort();
        }
        return route.continue();
      });
      await page.setViewportSize({ width: 1280, height: 900 });
      await page.goto(`http://127.0.0.1:9767/archives/${style}/index.html`);
      await expect(page.locator("body")).toHaveAttribute(
        "data-zim-template",
        style,
      );
      await expect(page.locator(".archive-item:visible")).toHaveCount(2);
      await page.evaluate(() => document.fonts.ready);
      expect(
        await page.evaluate(() => document.fonts.check("16px ZimSans")),
      ).toBe(true);
      await page.screenshot({
        path: `test-results/archive-templates/${style}-desktop.png`,
        fullPage: true,
      });
      await page.setViewportSize({ width: 390, height: 844 });
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth > innerWidth,
        ),
      ).toBe(false);
      await page.screenshot({
        path: `test-results/archive-templates/${style}-mobile.png`,
        fullPage: true,
      });
      await page
        .getByRole("searchbox", { name: "Search this collection", exact: true })
        .fill("molluscs");
      await expect(page.locator(".archive-item:visible")).toHaveCount(1);
      await expect(page.getByRole("status")).toHaveText("1 of 2 items");
      await expect(
        page.locator(".archive-item:visible .entry-kind"),
      ).toHaveText("Article");
      await page.getByRole("searchbox").fill("constructor");
      await expect(page.locator("#empty-state")).toBeVisible();
      await page.getByRole("button", { name: "Clear", exact: true }).click();
      await expect(page.locator(".archive-item:visible")).toHaveCount(2);
      await page
        .locator(".entry-detail, .entry-link")
        .filter({ hasText: "Molluscs of the forest" })
        .click();
      await expect(
        page.getByRole("navigation", { name: "Archive navigation" }),
      ).toHaveAttribute("data-zim-template", style);
      await page
        .getByRole("link", { name: "← Collection contents", exact: true })
        .click();
      await expect(
        page.getByRole("heading", {
          name: "A world worth exploring.",
          exact: true,
        }),
      ).toBeVisible();
      expect(outside).toEqual([]);
      await page.unroute("**/*");
      if (style === "screening_room") {
        const before = await (
          await request.get(`${api}/jobs/${job.id}`)
        ).json();
        const response = await request.post(`${api}/jobs/${job.id}/rebuild`, {
          data: { template: "midnight" },
        });
        expect(response.ok()).toBe(true);
        await expect
          .poll(async () => {
            const current = await (
              await request.get(`${api}/jobs/${job.id}`)
            ).json();
            if (current.status === "failed") throw new Error(current.error);
            return current.status;
          })
          .toBe("completed");
        const rebuilt = await (
          await request.get(`${api}/jobs/${job.id}`)
        ).json();
        expect(rebuilt.archive_template).toBe("midnight");
        expect(rebuilt.archive_revision).toBe(1);
        expect(rebuilt.requests).toBe(before.requests);
        expect(
          await readFile(resolve(content, "index.html"), "utf8"),
        ).toContain('data-zim-template="midnight"');
      }
    }
    expect(errors).toEqual([]);
  } finally {
    server.close();
    await request.post(`${api}/settings`, { data: preferences });
  }
});
