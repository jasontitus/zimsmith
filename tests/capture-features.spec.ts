import { test, expect, type Page } from "@playwright/test";
import { captureOptions, matchesRule, assetAllowed } from "../src/capture";

async function nativeApp(page: Page, completed = false) {
  await page.addInitScript((completed) => {
    const w = window as any;
    w.isTauri = true;
    w.featureCalls = [];
    w.importBytes = {};
    const config = {
      url: "https://example.test/",
      title: "Saved collection",
      kind: "website",
      max_items: 100,
      max_depth: 2,
      delay_ms: 1000,
      max_resource_mb: 256,
      language: "eng",
      description: "Fixture",
      creator: "Test",
      include_assets: true,
    };
    const make = (config: any, status: string) => ({
      id: "feature-fixture",
      config,
      status,
      phase: status === "completed" ? "package" : "scan",
      entries: [
        {
          id: "one",
          url: config.url,
          title: "First page",
          selected: true,
          state: status === "completed" ? "done" : "pending",
          depth: 0,
          size: 100,
          duration: null,
          error: null,
        },
      ],
      logs: [],
      error: null,
      retry_at: null,
      requests: 0,
      bytes: 100,
      created_at: 1,
      updated_at: 1,
      output_bytes: 1000,
      archive_template: "atlas",
    });
    w.featureJob = completed
      ? {
          ...make(config, "completed"),
          capture_report: {
            captured_items: 1,
            saved_assets: 8,
            omitted_assets: 2,
            excluded_assets: 1,
            failed_assets: 0,
            online_links: 3,
            subtitle_tracks: 0,
            caption_gaps: 0,
            search_limited: false,
            warnings: ["One source may be empty."],
          },
          validation: {
            checksum_verified: true,
            structure_verified: true,
            archive_entries: 20,
            checked_at: 1,
          },
        }
      : null;
    w.__TAURI_INTERNALS__ = {
      invoke: async (command: string, args: any = {}) => {
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
            auto_start_small_jobs: true,
            auto_start_item_limit: 100,
            archive_template: "atlas",
          };
        if (command === "archive_templates" || command === "get_shared_links")
          return [];
        if (command === "list_jobs") return w.featureJob ? [w.featureJob] : [];
        if (command === "get_job") return w.featureJob;
        w.featureCalls.push({ command, args });
        if (command === "create_job") {
          w.featureJob = make(args.config, "scanning");
          return w.featureJob;
        }
        if (command === "refresh_job") {
          w.featureJob = {
            ...w.featureJob,
            status: "scanning",
            phase: "scan",
            refresh: { compared: false, complete: false, changes: [] },
          };
          return w.featureJob;
        }
        if (command === "begin_import") {
          w.featureJob = {
            ...make(args.config, "importing"),
            phase: "import",
            import: { finished: false, file_count: args.files.length },
          };
          return w.featureJob;
        }
        if (command === "import_offset")
          return (w.importBytes[args.path] || "").length;
        if (command === "import_chunk") {
          const bytes = atob(args.data);
          w.importBytes[args.path] = (w.importBytes[args.path] || "") + bytes;
          return w.importBytes[args.path].length;
        }
        if (command === "finish_import") {
          w.featureJob = {
            ...w.featureJob,
            status: "review",
            phase: "scan",
            import: {
              finished: true,
              file_count: Object.keys(w.importBytes).length,
            },
          };
          return w.featureJob;
        }
        if (command === "cancel_import") return;
        throw new Error(`Unexpected ${command}`);
      },
    };
  }, completed);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
}

test("URL rules are literal globs and resource switches cover URLs and MIME types", () => {
  expect(
    matchesRule("/file[1].html", "https://example.test/file[1].html"),
  ).toBe(true);
  expect(matchesRule("/file[1].html", "https://example.test/file1.html")).toBe(
    false,
  );
  const config: any = {
    capture: { ...captureOptions({} as any), fonts: false, images: false },
  };
  expect(assetAllowed(config, "https://cdn.test/font.woff2?v=1")).toBe(false);
  expect(assetAllowed(config, "https://cdn.test/image", "image/png")).toBe(
    false,
  );
  expect(assetAllowed(config, "https://cdn.test/style.css")).toBe(true);
});

test("capture controls submit normalized URL rules, budget, and asset switches", async ({
  page,
}) => {
  await nativeApp(page);
  await page
    .getByLabel("Website URL", { exact: true })
    .fill("https://example.test/");
  await page
    .getByLabel("Archive title", { exact: true })
    .fill("Filtered capture");
  await page
    .getByRole("button", { name: "Crawl & archive settings", exact: true })
    .click();
  await page.getByLabel("Download budget (MB)", { exact: true }).fill("50");
  await page
    .getByLabel("Include page URLs", { exact: true })
    .fill(" /articles/* \n /guides/* ");
  await page
    .getByLabel("Exclude URLs", { exact: true })
    .fill("*/login*\n\n*/ads/*");
  await page.getByLabel("Fonts", { exact: true }).uncheck();
  await page.getByRole("button", { name: "Scan source", exact: true }).click();
  const c = await page.evaluate(
    () =>
      (window as any).featureCalls.find((c: any) => c.command === "create_job")
        .args.config.capture,
  );
  expect(c.max_download_mb).toBe(50);
  expect(c.fonts).toBe(false);
  expect(c.include_urls).toEqual(["/articles/*", "/guides/*"]);
  expect(c.exclude_urls).toEqual(["*/login*", "*/ads/*"]);
});

test("URL list import creates one scoped crawl and saved-file import streams through IPC", async ({
  page,
}) => {
  await nativeApp(page);
  const panel = page.locator(".import-panel");
  await panel.locator("summary").click();
  await panel.getByRole("button", { name: "URL list", exact: true }).click();
  await panel
    .getByLabel("Imported collection title", { exact: true })
    .fill("Reading list");
  await panel
    .getByLabel("Starting URLs, one per line", { exact: true })
    .fill(
      "example.test/guide/\nhttps://second.example/articles/\nexample.test/guide/",
    );
  await panel
    .getByRole("button", { name: "Scan URL list", exact: true })
    .click();
  const config = await page.evaluate(
    () =>
      (window as any).featureCalls.find((c: any) => c.command === "create_job")
        .args.config,
  );
  expect(config.url).toBe("https://example.test/guide/");
  expect(config.capture.seed_urls).toEqual([
    "https://second.example/articles/",
  ]);
  await page.reload();
  await page
    .getByRole("button", { name: "Create an archive", exact: true })
    .first()
    .click();
  await panel.locator("summary").click();
  const html = "<title>Saved page</title><p>Local bytes</p>";
  await panel.getByLabel("Choose saved files", { exact: true }).setInputFiles({
    name: "index.html",
    mimeType: "text/html",
    buffer: Buffer.from(html),
  });
  await panel
    .getByRole("button", { name: "Import & review", exact: true })
    .click();
  await expect(
    page.getByText("Imported local files", { exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(() => (window as any).importBytes["index.html"]),
  ).toBe(html);
  expect(
    await page.evaluate(
      () =>
        (window as any).featureCalls.filter(
          (c: any) => c.command === "import_chunk",
        ).length,
    ),
  ).toBe(1);
});

test("refresh and validation report are visible and incomplete scans do not claim removals", async ({
  page,
}) => {
  await nativeApp(page, true);
  await page
    .getByRole("button", { name: /My archives/ })
    .first()
    .click();
  await page
    .getByRole("button", { name: "Details for Saved collection", exact: true })
    .first()
    .click();
  await expect(
    page.getByRole("region", { name: "Capture & validation report" }),
  ).toContainText("Checksum and directory structure verified");
  await page
    .getByRole("button", { name: "Check for changes", exact: true })
    .click();
  await page.evaluate(() => {
    const w = window as any;
    w.featureJob.status = "review";
    w.featureJob.refresh = {
      compared: true,
      complete: false,
      changes: [
        { kind: "new", title: "New item", url: "https://example.test/new" },
        {
          kind: "not_seen",
          title: "Old item",
          url: "https://example.test/old",
        },
      ],
    };
  });
  await expect(
    page.getByRole("region", { name: "Changes since the previous capture" }),
  ).toContainText("1 new");
  await expect(
    page.getByRole("region", { name: "Changes since the previous capture" }),
  ).toContainText("don’t treat them as confirmed removals");
  expect(
    await page.evaluate(() =>
      (window as any).featureCalls.some((c: any) => c.command === "build_job"),
    ),
  ).toBe(false);
});
