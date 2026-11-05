import { test, expect } from "@playwright/test";
import { mkdir } from "node:fs/promises";

test("progress shows work, transfer bytes, cooldowns and errors with readable logs", async ({
  page,
}) => {
  const now = Math.floor(Date.now() / 1000);
  const entry = {
    id: "one",
    title: "First video",
    url: "https://example.com/one",
    depth: 0,
    selected: true,
    state: "pending",
    size: null,
    duration: null,
    error: null,
  };
  const job = {
    id: "progress-fixture",
    status: "scanning",
    phase: "scan",
    config: {
      title: "Progress collection",
      url: "https://example.com/",
      kind: "media",
      delay_ms: 2000,
    },
    entries: [entry, { ...entry, id: "two", title: "Second video" }],
    logs: [
      { time: now, level: "info", message: "Requesting source metadata" },
      {
        time: now,
        level: "warning",
        message: "The source is responding slowly",
      },
    ],
    updated_at: now,
    created_at: now,
    retry_at: null as number | null,
    error: null as string | null,
    bytes: 0,
    requests: 2,
    queued_pages: 0,
    auto_start_item_limit: null,
    progress: {
      label: "Requesting playlist metadata",
      completed: 2,
      total: null as number | null,
      unit: "items",
      received: 0,
      expected: null as number | null,
      updated_at: now,
    },
  };
  await page.route("http://127.0.0.1:8788/api/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    const value =
      path === "/api/jobs"
        ? [job]
        : path.startsWith("/api/jobs/")
          ? job
          : path === "/api/settings"
            ? {
                auto_start_small_jobs: false,
                auto_start_item_limit: 100,
                archive_template: "atlas",
              }
            : {
                platform: "ios",
                version: "0.1.0",
                local_execution: true,
                yt_dlp: { available: false, detail: "Native extractor" },
                zimwriterfs: { available: true, detail: "zimru" },
              };
    await route.fulfill({ json: value });
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await page.getByRole("button", { name: /Progress collection/ }).click();
  const progress = page.locator(".job-progress:not(.compact)");
  await expect(progress).toContainText("2 items found");
  await expect(progress.getByRole("progressbar")).not.toHaveAttribute(
    "aria-valuenow",
  );
  job.config.kind = "website";
  job.queued_pages = 4;
  await expect(progress).toContainText("2 items scanned · 4 pages queued");
  await expect(progress).toContainText(
    "Each queued page is opened and captured before review.",
  );
  job.phase = "download";
  job.status = "downloading";
  job.entries[0].state = "done";
  job.progress = {
    label: "Downloading video: Second video",
    completed: 1,
    total: 2,
    unit: "items",
    received: 1024 ** 2,
    expected: 10 * 1024 ** 2,
    updated_at: now,
  };
  await expect(progress).toContainText("1 of 2 items · 50%");
  await expect(progress).toContainText("1.0 MB of 10.0 MB");
  await expect(
    progress.getByRole("progressbar", { name: "Job stage progress" }),
  ).toHaveAttribute("aria-valuenow", "50");
  job.progress.received = 7 * 1024 ** 2;
  await expect(progress).toContainText("7.0 MB of 10.0 MB");
  const contrast = await page
    .locator(".log-line")
    .first()
    .evaluate((element) => {
      const rgb = (color: string) =>
        color
          .match(/[\d.]+/g)!
          .slice(0, 3)
          .map((n) => +n / 255);
      const lum = (values: number[]) =>
        values
          .map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4))
          .reduce((s, v, i) => s + v * [0.2126, 0.7152, 0.0722][i], 0);
      const bg = lum(
        rgb(getComputedStyle(element.parentElement!).backgroundColor),
      );
      return [element, ...element.querySelectorAll("time, .log-level")].map(
        (node) => {
          const fg = lum(rgb(getComputedStyle(node).color));
          return (Math.max(bg, fg) + 0.05) / (Math.min(bg, fg) + 0.05);
        },
      );
    });
  expect(contrast.every((ratio) => ratio >= 4.5)).toBe(true);
  expect(
    await page
      .locator(".log-line")
      .first()
      .evaluate((e) => parseFloat(getComputedStyle(e).fontSize)),
  ).toBeGreaterThanOrEqual(14);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    ),
  ).toBe(false);
  await mkdir("test-results/progress", { recursive: true });
  await page.screenshot({
    path: "test-results/progress/mobile.png",
    fullPage: true,
  });
  job.retry_at = Math.floor(Date.now() / 1000) + 60;
  await expect(progress).toContainText("Waiting for server cooldown");
  job.retry_at = null;
  job.status = "failed";
  job.error = "The media server returned HTTP 403";
  job.entries[1].state = "failed";
  job.logs.push({ time: now, level: "error", message: job.error });
  await expect(progress).toContainText("Needs attention");
  await expect(progress).not.toHaveClass(/active/);
  await expect(progress).toContainText("1 failed item");
  job.status = "packaging";
  job.error = null;
  job.phase = "package";
  job.progress = {
    ...job.progress,
    label: "Finalizing the ZIM directory and checksum",
    completed: 8,
    total: null,
    unit: "files",
    received: 0,
    expected: null,
  };
  await expect(progress).toContainText(
    "Finalizing the ZIM directory and checksum",
  );
  await expect(progress.getByRole("progressbar")).not.toHaveAttribute(
    "aria-valuenow",
  );
});
