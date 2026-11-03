import { test, expect, Page } from "@playwright/test";

async function failedPhone(page: Page, status = "failed") {
  await page.addInitScript((status) => {
    const w = window as any;
    w.isTauri = true;
    w.recoveryCalls = [];
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
    w.__TAURI_INTERNALS__ = {
      invoke: async (command: string) => {
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
        if (command === "list_jobs") return w.attempt ? [w.attempt] : [];
        if (command === "get_job") {
          if (!w.attempt) throw new Error("Job not found");
          return w.attempt;
        }
        w.recoveryCalls.push(command);
        if (command === "build_available_job") {
          w.attempt.status = "packaging";
          w.attempt.phase = "package";
          w.attempt.error = null;
          return w.attempt;
        }
        if (command === "delete_failed_job") {
          w.attempt = null;
          return true;
        }
        throw new Error(`Unexpected command: ${command}`);
      },
    };
  }, status);
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
    "delete_failed_job",
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
