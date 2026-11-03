import { test, expect } from "@playwright/test";
import { createServer } from "node:http";
import { mkdir } from "node:fs/promises";

test("pasting a shared YouTube playlist selects media scanning", async ({
  page,
}) => {
  let submitted: Record<string, unknown> | undefined;
  await page.route("http://127.0.0.1:8788/api/jobs", async (route) => {
    if (route.request().method() !== "POST") return route.continue();
    submitted = route.request().postDataJSON();
    await route.fulfill({
      status: 400,
      json: { error: "Metadata-only routing check" },
    });
  });
  await page.goto("/");
  const url =
    "https://youtube.com/playlist?list=PLalEW8Rh_rGk&si=B0TFoy5VtWkOP-LI";
  await page.getByLabel("Website URL", { exact: true }).fill(url);
  await expect(
    page.getByRole("button", { name: /Video collection/ }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.getByLabel("Collection or media URL", { exact: true }),
  ).toHaveValue(url);
  await expect(
    page.getByText("Media URL detected. We’ll scan this collection locally."),
  ).toBeVisible();
  await page
    .getByLabel("Archive title", { exact: true })
    .fill("YouTube playlist");
  await page.getByRole("button", { name: "Scan source", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Metadata-only routing check",
  );
  expect(submitted?.kind).toBe("media");
  expect(submitted?.url).toBe(url);
  await page
    .getByLabel("Collection or media URL", { exact: true })
    .fill("https://example.com/");
  await expect(page.getByLabel("Website URL", { exact: true })).toHaveValue(
    "https://example.com/",
  );
  await expect(page.getByRole("button", { name: /^Website/ })).toBeEnabled();
});

test("scan, review, confirm, and export a real offline collection", async ({
  page,
  request,
}) => {
  const preferences = await (
    await request.get("http://127.0.0.1:8788/api/settings")
  ).json();
  const assets: string[] = [];
  const server = createServer((request, response) => {
    const path = request.url;
    if (path === "/robots.txt") {
      response.end("User-agent: *\nAllow: /");
      return;
    }
    if (path === "/leaf.svg") {
      assets.push(path);
      response.setHeader("Content-Type", "image/svg+xml");
      response.end('<svg xmlns="http://www.w3.org/2000/svg"/>');
      return;
    }
    response.setHeader("Content-Type", "text/html");
    response.end(
      path === "/guide"
        ? "<html><head><title>A guide to wild places</title></head><body>Molluscs live in forests.</body></html>"
        : '<html><head><title>Field Notes</title></head><body>A small collection about nature.<a href="/guide">Read the guide</a><img src="/leaf.svg"></body></html>',
    );
  });
  await new Promise<void>((resolve) =>
    server.listen(9765, "127.0.0.1", resolve),
  );
  try {
    await page.goto("/");
    await expect(page.locator(".engine-status")).toContainText(
      "On-device engine",
    );
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
    await page.reload();
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await expect(
      page.getByLabel("Automatically start small jobs", { exact: true }),
    ).not.toBeChecked();
    await page
      .getByRole("button", { name: "Create an archive", exact: true })
      .click();
    await mkdir("test-results", { recursive: true });
    await page.screenshot({
      path: "test-results/home-desktop.png",
      fullPage: true,
    });
    await page
      .getByLabel("Website URL", { exact: true })
      .fill("http://127.0.0.1:9765/");
    await page
      .getByLabel("Archive title", { exact: true })
      .fill("Field Notes — offline");
    await page
      .getByRole("button", { name: "Scan source", exact: true })
      .click();
    await expect(
      page.getByText("Ready for review", { exact: true }),
    ).toBeVisible({ timeout: 30_000 });
    expect(assets.length).toBe(0);
    await expect(page.getByText("2 selected", { exact: true })).toBeVisible();
    await expect(
      page.getByText("A guide to wild places", { exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: "test-results/review-desktop.png",
      fullPage: true,
    });
    await page
      .getByRole("button", { name: "Review & build", exact: true })
      .click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await page
      .getByRole("button", { name: "Download & build ZIM", exact: true })
      .click();
    await expect(
      page.getByText("Your collection is ready to go.", { exact: true }),
    ).toBeVisible({ timeout: 45_000 });
    expect(assets.length).toBe(1);
    const downloadEvent = page.waitForEvent("download");
    await page
      .getByRole("button", { name: "Save ZIM file", exact: true })
      .click();
    const download = await downloadEvent;
    expect(download.suggestedFilename()).toMatch(/\.zim$/);
    await download.saveAs("test-results/field-notes.zim");
    await page.screenshot({
      path: "test-results/completed-desktop.png",
      fullPage: true,
    });
    await page.setViewportSize({ width: 390, height: 844 });
    await page
      .getByRole("button", { name: "Create an archive", exact: true })
      .click();
    await expect(
      page.getByRole("heading", { name: "Keep a piece of the web." }),
    ).toBeVisible();
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    );
    expect(overflow).toBe(false);
    await page.screenshot({
      path: "test-results/home-mobile.png",
      fullPage: true,
    });
  } finally {
    server.close();
    await request.post("http://127.0.0.1:8788/api/settings", {
      data: preferences,
    });
  }
});

test("saved threshold automatically builds a small collection without confirmation", async ({
  page,
  request,
}) => {
  const preferences = await (
    await request.get("http://127.0.0.1:8788/api/settings")
  ).json();
  let assets = 0;
  const server = createServer((req, response) => {
    if (req.url === "/robots.txt") {
      response.end("User-agent: *\nAllow: /");
      return;
    }
    if (req.url === "/image.svg") {
      assets++;
      response.setHeader("Content-Type", "image/svg+xml");
      response.end('<svg xmlns="http://www.w3.org/2000/svg"/>');
      return;
    }
    response.setHeader("Content-Type", "text/html");
    response.end(
      req.url === "/guide"
        ? "<title>Short guide</title><body>Local knowledge.</body>"
        : "<title>Small collection</title><body><a href='/guide'>Guide</a><img src='/image.svg'></body>",
    );
  });
  await new Promise<void>((resolve) =>
    server.listen(9766, "127.0.0.1", resolve),
  );
  try {
    await page.goto("/");
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await page
      .getByLabel("Automatically start small jobs", { exact: true })
      .check();
    await page
      .getByLabel("Maximum items for automatic download", { exact: true })
      .fill("2");
    await page
      .getByRole("button", { name: "Save download preferences", exact: true })
      .click();
    await expect(page.getByRole("status")).toContainText(
      "Download preferences saved",
    );
    await page.reload();
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await expect(
      page.getByLabel("Automatically start small jobs", { exact: true }),
    ).toBeChecked();
    await expect(
      page.getByLabel("Maximum items for automatic download", { exact: true }),
    ).toHaveValue("2");
    await page.setViewportSize({ width: 390, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth > innerWidth,
      ),
    ).toBe(false);
    await page.screenshot({
      path: "test-results/settings-mobile.png",
      fullPage: true,
    });
    await page
      .getByRole("button", { name: "Create an archive", exact: true })
      .click();
    await page
      .getByLabel("Website URL", { exact: true })
      .fill("http://127.0.0.1:9766/");
    await page
      .getByLabel("Archive title", { exact: true })
      .fill("Automatically built collection");
    await expect(
      page.getByText(
        /Complete, error-free scans of up to 2 items download and build automatically/,
      ),
    ).toBeVisible();
    await page
      .getByRole("button", { name: "Scan source", exact: true })
      .click();
    await expect(
      page.getByText("Your collection is ready to go.", { exact: true }),
    ).toBeVisible({ timeout: 45_000 });
    await expect(
      page.getByText(
        /Automatically approved 2 items using the saved threshold of 2/,
      ),
    ).toBeVisible();
    await expect(page.getByText("2 selected", { exact: true })).toBeVisible();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    expect(assets).toBe(1);
  } finally {
    server.close();
    await request.post("http://127.0.0.1:8788/api/settings", {
      data: preferences,
    });
  }
});
