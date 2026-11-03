import { test, expect, Page } from "@playwright/test";

async function mockAndroid(
  page: Page,
  options: { enabled?: boolean; active?: boolean } = {},
) {
  await page.addInitScript(
    ({ enabled, active }) => {
      const w = window as unknown as {
        isTauri: boolean;
        __TAURI_INTERNALS__: {
          invoke: (c: string, a?: Record<string, unknown>) => Promise<unknown>;
        };
        updateCalls: { command: string; args?: Record<string, unknown> }[];
        activeJob: boolean;
        failUpdate: boolean;
      };
      w.isTauri = true;
      w.activeJob = active;
      w.updateCalls = [];
      const release = {
        version: "0.1.2",
        version_code: 1002,
        url: "https://downloads.example/Zimsmith.apk",
        size_bytes: 36 * 1024 * 1024,
        notes: "More resilient crawling.\n<script>unsafe()</script>",
      };
      const status = {
        enabled,
        automatic: true,
        feed_url: "https://updates.example/android.json",
        current_version: "0.1.1",
        last_checked: null as number | null,
        retry_at: null,
        error: null,
        release: null as typeof release | null,
        dismissed_version: null as number | null,
      };
      w.__TAURI_INTERNALS__ = {
        invoke: async (command, args) => {
          if (command === "get_shared_links") return [];
          if (command.includes("update")) w.updateCalls.push({ command, args });
          if (command === "capabilities")
            return {
              platform: "android",
              version: "0.1.1",
              local_execution: true,
              yt_dlp: { available: false, detail: "RustyPipe" },
              zimwriterfs: { available: true, detail: "zimru" },
            };
          if (command === "preview_archive_template")
            return "<html><body>Template preview</body></html>";
          if (command === "archive_templates")
            return [{ id: "atlas", name: "Atlas", description: "Cards" }];
          if (command === "get_settings")
            return {
              auto_start_small_jobs: true,
              auto_start_item_limit: 100,
              archive_template: "atlas",
            };
          if (command === "list_jobs")
            return w.activeJob
              ? [
                  {
                    id: "active",
                    status: "downloading",
                    phase: "download",
                    entries: [],
                    config: {
                      title: "In progress",
                      url: "https://example.com/",
                      kind: "website",
                    },
                    created_at: 1,
                    updated_at: 1,
                    bytes: 0,
                    logs: [],
                    requests: 0,
                  },
                ]
              : [];
          if (command === "get_update_status") return { ...status };
          if (command === "check_app_updates") {
            status.release = release;
            status.last_checked = 1700000000;
            return { ...status };
          }
          if (command === "dismiss_app_update") {
            status.dismissed_version = 1002;
            return { ...status };
          }
          if (command === "configure_app_updates") {
            if (String(args?.feedUrl).startsWith("http:"))
              throw new Error("Update addresses must use HTTPS");
            if (status.feed_url !== args?.feedUrl) status.release = null;
            status.feed_url = String(args?.feedUrl);
            status.automatic = !!args?.automatic;
            return { ...status };
          }
          if (command === "download_app_update") {
            if (w.failUpdate) throw new Error("No browser is available");
            return;
          }
          throw new Error(`Unexpected command: ${command}`);
        },
      };
    },
    { enabled: options.enabled ?? true, active: options.active ?? false },
  );
}

test("direct Android checks updates and hands confirmed download to the native browser command", async ({
  page,
}) => {
  await mockAndroid(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  const offer = page.getByRole("region", { name: "Available app update" });
  await expect(offer).toContainText("0.1.2");
  await expect(offer).toContainText("<script>unsafe()</script>");
  await expect(offer.locator("script")).toHaveCount(0);
  await page
    .getByRole("button", { name: "Download update", exact: true })
    .click();
  await expect(page.getByRole("status")).toContainText("Opened your browser");
  const calls = await page.evaluate(
    () =>
      (
        window as unknown as {
          updateCalls: { command: string; args?: unknown }[];
        }
      ).updateCalls,
  );
  expect(calls.find((c) => c.command === "check_app_updates")?.args).toEqual({
    manual: false,
  });
  expect(calls.find((c) => c.command === "download_app_update")?.args).toEqual(
    {},
  );
  await page.getByRole("button", { name: "Later", exact: true }).click();
  await expect(offer).toHaveCount(0);
  await page
    .getByRole("button", { name: "Open settings", exact: true })
    .click();
  await expect(
    page.getByRole("region", { name: "App updates", exact: true }),
  ).toBeVisible();
  await page.getByText("Advanced: update source", { exact: true }).click();
  await page
    .getByLabel("HTTPS release feed")
    .fill("http://insecure.example/feed");
  await page.getByRole("button", { name: "Save update settings" }).click();
  await expect(page.getByRole("alert")).toContainText("HTTPS");
  await page
    .getByLabel("HTTPS release feed")
    .fill("https://new-host.example/updates/android.json");
  await page.getByLabel("Check automatically").uncheck();
  await page.getByRole("button", { name: "Save update settings" }).click();
  await expect(page.getByRole("status")).toContainText("preferences saved");
  await expect(offer).toHaveCount(0);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    ),
  ).toBe(false);
});

test("running crawl defers browser handoff until paused", async ({ page }) => {
  await mockAndroid(page, { active: true });
  await page.goto("/");
  const download = page.getByRole("button", {
    name: "Download update",
    exact: true,
  });
  await expect(download).toBeDisabled();
  await expect(
    page.getByRole("region", { name: "Available app update" }),
  ).toContainText("Pause running jobs");
  await page.evaluate(() => {
    (window as unknown as { activeJob: boolean }).activeJob = false;
  });
  await expect(download).toBeEnabled();
  await page.evaluate(() => {
    (window as unknown as { failUpdate: boolean }).failUpdate = true;
  });
  await download.click();
  await expect(page.getByRole("alert")).toContainText(
    "No browser is available",
  );
});

test("store Android build never initiates direct update checks", async ({
  page,
}) => {
  await mockAndroid(page, { enabled: false });
  await page.goto("/");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (
          window as unknown as { updateCalls: { command: string }[] }
        ).updateCalls.map((c) => c.command),
      ),
    )
    .toContain("get_update_status");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(
    page.getByRole("region", { name: "App updates", exact: true }),
  ).toHaveCount(0);
  const commands = await page.evaluate(() =>
    (
      window as unknown as { updateCalls: { command: string }[] }
    ).updateCalls.map((c) => c.command),
  );
  expect(commands).not.toContain("check_app_updates");
});
