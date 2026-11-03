import { test, expect } from "@playwright/test";
import { mkdir } from "node:fs/promises";

test("phone controls retain scale and finished archives use native save/open commands", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.addInitScript(() => {
    const state = window as unknown as {
      isTauri: boolean;
      __TAURI_INTERNALS__: {
        invoke: (command: string, args: unknown) => Promise<unknown>;
      };
      nativeCalls: string[];
      noViewer: boolean;
      rebuildArgs: unknown;
    };
    state.isTauri = true;
    state.nativeCalls = [];
    const job = {
      id: "fixture-archive",
      status: "completed",
      phase: "package",
      entries: [],
      config: {
        title: "Field Notes",
        url: "https://example.com/",
        kind: "website",
        delay_ms: 2000,
      },
      created_at: 1,
      updated_at: 1,
      bytes: 1024,
      output_bytes: 1024,
      retry_at: null,
      error: null,
      logs: [],
      requests: 1,
      auto_start_item_limit: null,
    };
    state.__TAURI_INTERNALS__ = {
      invoke: async (command, args) => {
        if (command === "get_shared_links") return [];
        state.nativeCalls.push(command);
        if (command === "archive_templates")
          return [
            { id: "atlas", name: "Atlas", description: "Cards" },
            {
              id: "screening_room",
              name: "Screening Room",
              description: "Video gallery",
            },
          ];
        if (command === "rebuild_job") {
          state.rebuildArgs = args;
          job.status = "packaging";
          return job;
        }
        if (command === "list_jobs") return [job];
        if (command === "get_job") return job;
        if (command === "get_settings")
          return {
            auto_start_small_jobs: true,
            auto_start_item_limit: 100,
            archive_template: "atlas",
          };
        if (command === "capabilities")
          return {
            platform: "ios",
            version: "0.1.0",
            local_execution: true,
            yt_dlp: { available: false, detail: "Not bundled" },
            zimwriterfs: { available: true, detail: "zimru" },
          };
        if (command === "export_archive")
          return "Saved to your chosen folder: Field Notes.zim";
        if (command === "open_archive") {
          if (state.noViewer)
            throw new Error(
              "No ZIM viewer is installed. Install a compatible viewer, then tap Open ZIM again.",
            );
          return;
        }
        throw new Error(`Unexpected command: ${command}`);
      },
    };
  });
  await page.goto("/");
  const viewport = await page
    .locator('meta[name="viewport"]')
    .getAttribute("content");
  expect(viewport).toContain("maximum-scale=1");
  expect(viewport).toContain("user-scalable=no");
  await page
    .getByLabel("Website URL", { exact: true })
    .fill("https://example.com/");
  const inputSizes = await page
    .locator('input:not([type="checkbox"])')
    .evaluateAll((inputs) =>
      inputs.map((input) => parseFloat(getComputedStyle(input).fontSize)),
    );
  expect(inputSizes.every((size) => size >= 16)).toBe(true);
  expect(await page.evaluate(() => visualViewport?.scale)).toBe(1);
  await page
    .getByRole("button", { name: "Open Field Notes ZIM", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { nativeCalls: string[] }).nativeCalls.filter(
            (c) => c === "open_archive",
          ).length,
      ),
    )
    .toBe(1);
  await page
    .getByRole("button", { name: "Details for Field Notes", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Save ZIM file", exact: true })
    .click();
  await expect(page.getByRole("status")).toContainText(
    "Saved to your chosen folder: Field Notes.zim",
  );
  await page.getByRole("button", { name: "Open ZIM", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { nativeCalls: string[] }).nativeCalls.filter(
            (c) => c === "open_archive",
          ).length,
      ),
    )
    .toBe(2);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    ),
  ).toBe(false);
  await mkdir("test-results/native-files", { recursive: true });
  await page.screenshot({
    path: "test-results/native-files/completed-mobile.png",
    fullPage: true,
  });
  await page.evaluate(() => {
    (window as unknown as { noViewer: boolean }).noViewer = true;
  });
  await page.getByRole("button", { name: "Open ZIM", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText(
    "No ZIM viewer is installed",
  );
  await page
    .getByLabel("Archive template", { exact: true })
    .selectOption("screening_room");
  await page.getByRole("button", { name: "Rebuild ZIM", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { rebuildArgs: unknown }).rebuildArgs,
      ),
    )
    .toEqual({ id: "fixture-archive", template: "screening_room" });
  await expect(
    page.getByRole("heading", { name: "Your collection is ready to go." }),
  ).toHaveCount(0);
  await expect(page.getByRole("status")).toContainText(
    "Rebuilding from downloaded files",
  );
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("completed native YouTube collection can repair its original audio", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as unknown as {
      isTauri: boolean;
      __TAURI_INTERNALS__: {
        invoke: (command: string, args?: unknown) => Promise<unknown>;
      };
      repairArgs: unknown;
    };
    w.isTauri = true;
    const job = {
      id: "youtube-fixture",
      status: "completed",
      phase: "package",
      entries: [
        {
          id: "video-one",
          title: "Video one",
          url: "https://www.youtube.com/watch?v=fixture1234",
          selected: true,
          state: "done",
          size: 100,
          duration: 1,
          depth: 0,
          error: null,
        },
      ],
      config: {
        title: "Video collection",
        url: "https://www.youtube.com/playlist?list=fixture",
        kind: "media",
        delay_ms: 1000,
        language: "eng",
      },
      created_at: 1,
      updated_at: 1,
      bytes: 100,
      output_bytes: 200,
      retry_at: null,
      error: null,
      logs: [],
      requests: 1,
      auto_start_item_limit: null,
      archive_template: "screening_room",
    };
    w.__TAURI_INTERNALS__ = {
      invoke: async (command, args) => {
        if (command === "get_shared_links") return [];
        if (command === "archive_templates")
          return [
            {
              id: "screening_room",
              name: "Screening Room",
              description: "Gallery",
            },
          ];
        if (command === "get_settings")
          return {
            auto_start_small_jobs: true,
            auto_start_item_limit: 100,
            archive_template: "screening_room",
          };
        if (command === "capabilities")
          return {
            platform: "ios",
            version: "0.1.4",
            local_execution: true,
            yt_dlp: { available: false, detail: "RustyPipe" },
            zimwriterfs: { available: true, detail: "zimru" },
          };
        if (command === "list_jobs") return [job];
        if (command === "get_job") return job;
        if (command === "repair_youtube_audio") {
          w.repairArgs = args;
          job.status = "downloading";
          job.phase = "download";
          return job;
        }
        throw new Error(`Unexpected command: ${command}`);
      },
    };
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await page
    .getByRole("button", { name: "Details for Video collection", exact: true })
    .click();
  const repair = page.getByRole("button", {
    name: "Repair original audio",
    exact: true,
  });
  await expect(repair).toBeVisible();
  await expect(
    page.getByText(/Matching saved video files are reused/),
  ).toBeVisible();
  await repair.click();
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { repairArgs: unknown }).repairArgs,
      ),
    )
    .toEqual({ id: "youtube-fixture" });
  await expect(page.getByRole("status")).toContainText(
    "Repairing original audio",
  );
  await expect(repair).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
});
