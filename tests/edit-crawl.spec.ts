import { test, expect } from "@playwright/test";
import { createServer } from "node:http";

const api = "http://127.0.0.1:8788/api";

test("hostname input adds HTTPS and still detects shared media URLs", async ({
  page,
}) => {
  const submitted: Record<string, unknown>[] = [];
  await page.route(`${api}/jobs`, async (route) => {
    if (route.request().method() !== "POST") return route.continue();
    submitted.push(route.request().postDataJSON());
    await route.fulfill({
      status: 400,
      json: { error: "Normalization verified" },
    });
  });
  await page.goto("/");
  const source = page.locator("#source-url");
  await source.fill("  CNN.com/world?sort=latest#top  ");
  await page.getByLabel("Archive title", { exact: true }).focus();
  await expect(source).toHaveValue("https://cnn.com/world?sort=latest");
  await expect(page.getByLabel("Archive title", { exact: true })).toHaveValue(
    "cnn.com",
  );
  await expect(source).toHaveAttribute("inputmode", "url");
  await expect(source).toHaveAttribute("autocapitalize", "none");
  await page.getByRole("button", { name: "Scan source", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Normalization verified");
  expect(submitted[0].url).toBe("https://cnn.com/world?sort=latest");
  expect(submitted[0].kind).toBe("website");
  await source.fill("youtube.com/playlist?list=PLalEW8Rh_rGk");
  await page.getByLabel("Archive title", { exact: true }).focus();
  await expect(source).toHaveValue(
    "https://youtube.com/playlist?list=PLalEW8Rh_rGk",
  );
  await expect(
    page.getByRole("button", { name: /Video collection/ }),
  ).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "Scan source", exact: true }).click();
  await expect.poll(() => submitted.length).toBe(2);
  expect(submitted[1].kind).toBe("media");
  await page
    .getByLabel("Archive title", { exact: true })
    .fill("URL validation check");
  await source.fill("javascript:alert(1)");
  await page.getByRole("button", { name: "Scan source", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("Only HTTP and HTTPS");
  expect(submitted).toHaveLength(2);
});

test("failed crawls can be edited, cancelled, and retried under the same job", async ({
  page,
  request,
}) => {
  const preferences = await (await request.get(`${api}/settings`)).json();
  const server = createServer((req, response) => {
    if (req.url === "/robots.txt") {
      response.end("User-agent: *\nAllow: /");
      return;
    }
    if (req.url === "/missing") {
      response.writeHead(404).end("Missing");
      return;
    }
    response.setHeader("Content-Type", "text/html");
    response.end(
      "<html><title>Recovered source</title><body>A page saved after editing.</body></html>",
    );
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("No fixture address");
  const origin = `http://127.0.0.1:${address.port}`;
  try {
    await request.post(`${api}/settings`, {
      data: {
        ...preferences,
        auto_start_small_jobs: true,
        auto_start_item_limit: 100,
      },
    });
    const response = await request.post(`${api}/jobs`, {
      data: {
        url: `${origin}/missing`,
        title: "Failed crawl to edit",
        kind: "website",
        max_items: 5,
        max_depth: 1,
        delay_ms: 1000,
        max_resource_mb: 2,
        language: "eng",
        description: "Edit fixture",
        creator: "Test",
        include_assets: false,
      },
    });
    expect(response.ok()).toBe(true);
    const created = await response.json();
    await expect
      .poll(
        async () =>
          (await (await request.get(`${api}/jobs/${created.id}`)).json())
            .status,
      )
      .toBe("failed");
    let edits = 0,
      newJobs = 0;
    await page.route(`${api}/jobs/${created.id}/edit`, async (route) => {
      edits++;
      await route.continue();
    });
    page.on("request", (req) => {
      if (req.url() === `${api}/jobs` && req.method() === "POST") newJobs++;
    });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto("/");
    await page
      .locator(".job-row")
      .filter({ hasText: "Failed crawl to edit" })
      .click();
    await page.getByRole("button", { name: "Edit crawl", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: "Edit your crawl.", exact: true }),
    ).toBeVisible();
    await expect(page.locator("#source-url")).toHaveValue(`${origin}/missing`);
    await expect(page.locator("#limit")).toHaveValue("5");
    await expect(page.getByLabel("Link depth", { exact: true })).toHaveValue(
      "1",
    );
    await page.locator("#source-url").fill("cnn.com");
    await page.getByLabel("Archive title", { exact: true }).focus();
    await expect(page.locator("#source-url")).toHaveValue("https://cnn.com/");
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth > innerWidth,
      ),
    ).toBe(false);
    await page.screenshot({
      path: test.info().outputPath("edit-mobile.png"),
      fullPage: true,
    });
    await page
      .getByRole("button", { name: "Cancel edit", exact: true })
      .click();
    expect(edits).toBe(0);
    expect(
      (await (await request.get(`${api}/jobs/${created.id}`)).json()).config
        .url,
    ).toBe(`${origin}/missing`);
    await page.getByRole("button", { name: "Edit crawl", exact: true }).click();
    await page.locator("#source-url").fill(`${origin}/`);
    await page
      .getByLabel("Archive title", { exact: true })
      .fill("Recovered edited collection");
    await page.locator("#limit").fill("10");
    const depth = page.getByLabel("Link depth", { exact: true });
    await depth.fill("0");
    await depth.press("Backspace");
    await expect(depth).toHaveValue("");
    await depth.fill("0");
    const delay = page.getByLabel("Delay between page requests (seconds)", {
      exact: true,
    });
    await delay.press("Backspace");
    await expect(delay).toHaveValue("");
    await delay.fill("1.5");
    await page
      .getByRole("button", { name: "Save & retry", exact: true })
      .click();
    await expect(
      page.getByRole("heading", {
        name: "Your collection is ready to go.",
        exact: true,
      }),
    ).toBeVisible({ timeout: 35000 });
    expect(edits).toBe(1);
    expect(newJobs).toBe(0);
    const completed = await (
      await request.get(`${api}/jobs/${created.id}`)
    ).json();
    expect(completed.status).toBe("completed");
    expect(completed.config).toMatchObject({
      url: `${origin}/`,
      title: "Recovered edited collection",
      max_items: 10,
      max_depth: 0,
      delay_ms: 1500,
    });
    expect(completed.entries).toHaveLength(1);
    expect(completed.error).toBeNull();
  } finally {
    server.close();
    await request.post(`${api}/settings`, { data: preferences });
  }
});
