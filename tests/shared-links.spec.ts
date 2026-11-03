import { test, expect, Page } from "@playwright/test";

async function sharedPhone(page: Page, initial: boolean) {
  await page.addInitScript((initial) => {
    const w = window as unknown as {
      isTauri: boolean;
      __TAURI_INTERNALS__: { invoke: (c: string, a?: any) => Promise<any> };
      shares: any[];
      sharedCalls: string[];
      created: any;
      rejectCreate: boolean;
    };
    const link = {
      id: "share-one",
      url: "https://youtu.be/fixture1234",
      title: "Shared video",
      error: null,
    };
    w.isTauri = true;
    w.shares = initial ? [link] : [];
    w.sharedCalls = [];
    w.rejectCreate = false;
    let job: any;
    w.__TAURI_INTERNALS__ = {
      invoke: async (c, args) => {
        w.sharedCalls.push(c);
        if (c === "capabilities")
          return {
            platform: "ios",
            version: "test",
            local_execution: true,
            yt_dlp: {},
            zimwriterfs: {},
          };
        if (c === "archive_templates") return [];
        if (c === "get_settings")
          return {
            auto_start_small_jobs: false,
            auto_start_item_limit: 100,
            archive_template: "atlas",
          };
        if (c === "get_shared_links") return w.shares;
        if (c === "dismiss_shared_link") {
          w.shares = w.shares.filter((l) => l.id !== args.id);
          return;
        }
        if (c === "list_jobs") return job ? [job] : [];
        if (c === "get_job") return job;
        if (c === "create_job") {
          if (w.rejectCreate) throw new Error("Could not start scan");
          w.created = args.config;
          job = {
            id: "created",
            config: args.config,
            status: "scanning",
            phase: "scan",
            entries: [],
            logs: [],
            progress: {
              completed: 0,
              total: null,
              label: "Scanning",
              unit: "items",
              updated_at: 1,
            },
            created_at: 1,
            updated_at: 1,
            bytes: 0,
            requests: 0,
            error: null,
          };
          return job;
        }
        throw new Error(`Unexpected command: ${c}`);
      },
    };
  }, initial);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
}

test("cold share prefills media without starting a crawl and is cleared only after creation succeeds", async ({
  page,
}) => {
  await sharedPhone(page, true);
  await expect(page.getByText("Reviewing link", { exact: true })).toHaveCount(
    0,
  );
  await expect(
    page.getByLabel("Collection or media URL", { exact: true }),
  ).toHaveValue("https://youtu.be/fixture1234");
  await expect(page.getByLabel("Archive title", { exact: true })).toHaveValue(
    "Shared video",
  );
  expect(
    await page.evaluate(() =>
      (window as any).sharedCalls.includes("create_job"),
    ),
  ).toBe(false);
  await page.evaluate(() => {
    (window as any).rejectCreate = true;
  });
  await page
    .getByRole("button", { name: "Scan shared link", exact: true })
    .click();
  await expect(page.getByRole("alert")).toContainText("Could not start scan");
  expect(await page.evaluate(() => (window as any).shares.length)).toBe(1);
  await page.reload();
  await expect(
    page.getByLabel("Collection or media URL", { exact: true }),
  ).toHaveValue("https://youtu.be/fixture1234");
  await page
    .getByRole("button", { name: "Scan shared link", exact: true })
    .click();
  await expect
    .poll(() => page.evaluate(() => (window as any).shares.length))
    .toBe(0);
  expect(await page.evaluate(() => (window as any).created.kind)).toBe("media");
});

test("a new share preserves an existing form until the user reviews it", async ({
  page,
}) => {
  await sharedPhone(page, false);
  await page
    .getByLabel("Website URL", { exact: true })
    .fill("https://example.com/my-draft");
  await page.getByLabel("Archive title", { exact: true }).fill("My draft");
  await page.evaluate(() => {
    (window as any).shares = [
      {
        id: "share-two",
        url: "https://other.example/docs/",
        title: "Other docs",
        error: null,
      },
    ];
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await expect(
    page.getByRole("button", { name: "Set up crawl", exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel("Website URL", { exact: true })).toHaveValue(
    "https://example.com/my-draft",
  );
  await page.getByRole("button", { name: "Set up crawl", exact: true }).click();
  await expect(page.getByLabel("Website URL", { exact: true })).toHaveValue(
    "https://other.example/docs/",
  );
  await expect(page.getByLabel("Archive title", { exact: true })).toHaveValue(
    "Other docs",
  );
  await page.getByRole("button", { name: "Dismiss Other docs" }).click();
  await expect(page.getByRole("region", { name: "Shared links" })).toHaveCount(
    0,
  );
  expect(await page.evaluate(() => (window as any).created)).toBeUndefined();
});

test("invalid shared text is dismissible and never becomes a crawl", async ({
  page,
}) => {
  await sharedPhone(page, false);
  await page.evaluate(() => {
    (window as any).shares = [
      {
        id: "invalid",
        url: null,
        title: "Unusable shared link",
        error: "Share one website link at a time.",
      },
    ];
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await expect(
    page.getByText("Share one website link at a time."),
  ).toBeVisible();
  await expect(page.getByLabel("Website URL", { exact: true })).toHaveValue("");
  await expect(
    page.getByRole("button", { name: "Set up crawl", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Dismiss Unusable shared link" })
    .click();
  expect(await page.evaluate(() => (window as any).shares.length)).toBe(0);
});
