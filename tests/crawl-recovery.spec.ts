import { test, expect, Page } from "@playwright/test";

async function failedPhone(
  page: Page,
  status = "failed",
  itemCount = 1,
  extraJobs = false,
) {
  await page.addInitScript(
    ({ status, itemCount, extraJobs }) => {
      const w = window as any;
      w.isTauri = true;
      w.recoveryCalls = [];
      w.deleted = [];
      w.attempt = {
        id: "failed-fixture",
        status,
        phase: "download",
        archive_template: "atlas",
        config: {
          url: "https://example.test/",
          title: "Interrupted collection",
          kind: "website",
          delay_ms: 1000,
          max_resource_mb: 2,
          max_items: 10,
          max_depth: 1,
          language: "eng",
          description: "Fixture",
          creator: "Test",
          include_assets: true,
        },
        entries: [
          {
            id: "home",
            selected: true,
            state: "pending",
            title: "Home",
            url: "https://example.test/",
            depth: 0,
            size: 100,
            duration: null,
            error: null,
          },
        ],
        logs: [],
        error: status === "failed" ? "Cannot safely read CDN robots.txt" : null,
        requests: 9,
        bytes: 100,
        created_at: 1,
        updated_at: 1,
        retry_at: null,
        output: null,
        output_bytes: null,
        asset_summary: { downloaded: 8, missing: 8, omitted: 0 },
      };
      if (itemCount > 1) {
        const first = w.attempt.entries[0];
        w.attempt.entries = Array.from({ length: itemCount }, (_, i) => ({
          ...first,
          id: String(i),
          title: `Long collection item ${i} with a descriptive title that wraps across lines`,
          url: `https://example.test/${"long-path-segment/".repeat(7)}${i}`,
        }));
      }
      w.otherJobs = extraJobs
        ? [
            {
              ...w.attempt,
              id: "second-failed",
              config: { ...w.attempt.config, title: "Failed build two" },
            },
            {
              ...w.attempt,
              id: "paused-build",
              status: "paused",
              config: { ...w.attempt.config, title: "Paused build" },
            },
            {
              ...w.attempt,
              id: "running-build",
              status: "packaging",
              config: { ...w.attempt.config, title: "Running build" },
            },
          ]
        : [];
      w.__TAURI_INTERNALS__ = {
        invoke: async (
          command: string,
          args: { id: string; expectedStatus?: string },
        ) => {
          if (command === "capabilities")
            return {
              platform: "ios",
              version: "test",
              local_execution: true,
              yt_dlp: {},
              zimwriterfs: {},
            };
          if (command === "get_settings")
            return {
              auto_start_small_jobs: false,
              auto_start_item_limit: 100,
              archive_template: "atlas",
            };
          if (command === "get_shared_links" || command === "archive_templates")
            return [];
          if (command === "list_jobs")
            return [...(w.attempt ? [w.attempt] : []), ...w.otherJobs];
          if (command === "get_job") {
            if (!w.attempt) throw new Error("Job not found");
            return w.attempt;
          }
          w.recoveryCalls.push(command);
          if (command === "resume_job") {
            w.attempt.status = "downloading";
            w.attempt.error = null;
            return w.attempt;
          }
          if (command === "build_available_job") {
            w.attempt.status = "packaging";
            w.attempt.phase = "package";
            w.attempt.error = null;
            return w.attempt;
          }
          if (command === "delete_job") {
            w.deleted.push(args);
            if (args.id === w.attempt?.id) w.attempt = null;
            else
              w.otherJobs = w.otherJobs.filter(
                (j: { id: string }) => j.id !== args.id,
              );
            return true;
          }
          throw new Error(`Unexpected command: ${command}`);
        },
      };
    },
    { status, itemCount, extraJobs },
  );
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await page
    .getByRole("button", { name: /Activity/ })
    .first()
    .click();
  await page
    .locator(".job-row")
    .filter({ hasText: "Interrupted collection" })
    .click();
}

test("phone retry is beside the error and resumes the saved crawl", async ({
  page,
}) => {
  await failedPhone(page);
  const retry = page.getByRole("button", {
    name: "Retry & resume",
    exact: true,
  });
  await expect(retry).toHaveCount(1);
  await expect(retry).toBeInViewport();
  await expect(
    page.getByText(
      "Saved files are reused. Only unfinished items are retried.",
    ),
  ).toBeVisible();
  await retry.click();
  expect(await page.evaluate(() => (window as any).recoveryCalls)).toEqual([
    "resume_job",
  ]);
  await expect(retry).toHaveCount(0);
});

test("Activity exposes row deletion and clears only the confirmed failed builds", async ({
  page,
}) => {
  await failedPhone(page, "failed", 1, true);
  await page.getByRole("button", { name: "All activity", exact: true }).click();
  const deleteRow = page.getByRole("button", {
    name: "Delete failed attempt for Interrupted collection",
  });
  await expect(deleteRow).toBeVisible();
  await deleteRow.click();
  const single = page.getByRole("dialog", { name: "Delete failed attempt?" });
  await expect(single).toContainText("Interrupted collection");
  await single.getByRole("button", { name: "Keep attempt" }).click();
  const paused = page.getByRole("button", {
    name: "Delete build for Paused build",
  });
  await expect(paused).toBeVisible();
  await expect(
    page
      .locator(".job-row-container")
      .filter({ hasText: "Running build" })
      .getByRole("button", { name: /delete/i }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Clear failed builds (2)", exact: true })
    .click();
  const bulk = page.getByRole("dialog", { name: "Clear failed builds?" });
  await expect(bulk).toContainText("Remove 2 failed builds");
  await bulk.getByRole("button", { name: "Delete 2 failed builds" }).click();
  await expect(deleteRow).toHaveCount(0);
  await expect(paused).toBeVisible();
  expect(await page.evaluate(() => (window as any).deleted)).toEqual([
    { id: "failed-fixture", expectedStatus: "failed" },
    { id: "second-failed", expectedStatus: "failed" },
  ]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(
    390,
  );
});

test("phone review uses readable rows and page scrolling with working selection and filtering", async ({
  page,
}) => {
  await failedPhone(page, "review", 24);
  const list = page.getByRole("list", { name: "Collection items" });
  await expect(list.getByRole("listitem")).toHaveCount(24);
  for (const width of [390, 320]) {
    await page.setViewportSize({ width, height: 844 });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await expect(list).toHaveCSS("overflow-y", "visible");
    const choice = list.locator(".entry-selector").first();
    const bounds = await choice.boundingBox();
    expect(bounds!.width).toBeGreaterThanOrEqual(44);
    expect(bounds!.height).toBeGreaterThanOrEqual(44);
  }
  const selected = list.getByRole("checkbox").first();
  await selected.uncheck();
  await expect(selected).not.toBeChecked();
  await expect(page.getByText("23 selected", { exact: true })).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await list.getByRole("listitem").first().scrollIntoViewIfNeeded();
  await page.screenshot({ path: "test-results/phone-collection-review.png" });
  await page
    .getByRole("textbox", { name: "Filter collection" })
    .fill("item 23 ");
  await expect(list.getByRole("listitem")).toHaveCount(1);
  await expect(list).toContainText("Long collection item 23");
});

test("delete asks first, cancellation preserves attempt, confirmation removes it", async ({
  page,
}) => {
  await failedPhone(page);
  await page
    .getByRole("button", { name: "Delete failed attempt", exact: true })
    .click();
  const dialog = page.getByRole("dialog", { name: "Delete failed attempt?" });
  await expect(dialog).toContainText("Exported ZIM copies stay unchanged");
  await dialog.getByRole("button", { name: "Keep attempt" }).click();
  expect(await page.evaluate(() => (window as any).recoveryCalls)).toEqual([]);
  await page
    .getByRole("button", { name: "Delete failed attempt", exact: true })
    .click();
  await dialog
    .getByRole("button", { name: "Delete attempt", exact: true })
    .click();
  await expect(dialog).toHaveCount(0);
  await expect(
    page.locator(".job-row").filter({ hasText: "Interrupted collection" }),
  ).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).recoveryCalls)).toEqual([
    "delete_job",
  ]);
  await expect(
    page.getByText("Failed attempt and cached files deleted.", { exact: true }),
  ).toBeVisible();
});

test("paused download offers explicit partial build and explains missing assets", async ({
  page,
}) => {
  await failedPhone(page, "paused");
  await expect(
    page.getByRole("button", { name: "Delete failed attempt", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Delete build", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Finish with what’s downloaded" }),
  ).toContainText("8 assets saved · 8 not downloaded");
  await expect(
    page.getByRole("region", { name: "Finish with what’s downloaded" }),
  ).toContainText("capture report");
  await page
    .getByRole("button", { name: "Build with available files", exact: true })
    .click();
  expect(await page.evaluate(() => (window as any).recoveryCalls)).toEqual([
    "build_available_job",
  ]);
  await expect(
    page.getByRole("button", {
      name: "Build with available files",
      exact: true,
    }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("heading", { name: "Interrupted collection", exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    ),
  ).toBe(false);
});
