import { test, expect } from "@playwright/test";

test("compression preference persists, validates its range, and preserves other settings drafts", async ({
  page,
  request,
}, testInfo) => {
  const endpoint = "http://127.0.0.1:8788/api/settings";
  const previous = await (await request.get(endpoint)).json();
  try {
    expect(
      (
        await request.post(endpoint, { data: { ...previous, zstd_level: 3 } })
      ).ok(),
    ).toBe(true);
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto("/");
    await page
      .getByRole("button", { name: "Open settings", exact: true })
      .click();
    const level = page.getByLabel("Zstandard compression level", {
      exact: true,
    });
    await expect(level).toHaveValue("3");
    for (const invalid of ["0", "23", "1.5"]) {
      await level.fill(invalid);
      expect(
        await level.evaluate((input: HTMLInputElement) =>
          input.checkValidity(),
        ),
      ).toBe(false);
    }
    await level.fill("9");
    const otherStyle =
      previous.archive_template === "midnight" ? "Atlas" : "Midnight";
    await page.getByRole("radio", { name: otherStyle, exact: true }).check();
    await page
      .getByRole("button", { name: "Save compression preference" })
      .click();
    await expect(page.getByRole("status")).toContainText(
      "Compression preference saved",
    );
    const saved = await (await request.get(endpoint)).json();
    expect(saved.zstd_level).toBe(9);
    expect(saved.archive_template).toBe(previous.archive_template);
    await level.fill("6");
    await page
      .getByRole("button", { name: "Save archive style", exact: true })
      .click();
    await expect(page.getByRole("status")).toContainText("Archive style saved");
    expect((await (await request.get(endpoint)).json()).zstd_level).toBe(9);
    await expect(level).toHaveValue("6");
    await page.reload();
    await page
      .getByRole("button", { name: "Open settings", exact: true })
      .click();
    await expect(level).toHaveValue("9");
    await page
      .locator(".compression-preferences")
      .screenshot({ path: testInfo.outputPath("compression-mobile.png") });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth > innerWidth,
      ),
    ).toBe(false);
  } finally {
    await request.post(endpoint, { data: previous });
  }
});
