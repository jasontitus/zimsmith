import { test, expect } from "@playwright/test";
import { DOMParser } from "linkedom/worker";
import { staticPresentation } from "../src/browser/snapshot";
import { rewriteCss } from "../src/browser/css";

test("CSS parsing preserves embedded SVGs and fragments without phantom downloads", async () => {
  const svg = `url("data:image/svg+xml,%3Csvg%3E%3Cg mask='url(%23mask)'/%3E")`;
  const input = `/* url(fake.png) */ a{mask:${svg};filter:url(#local);background:url("real (one).svg#shape");content:'url(text.png)'} @import "theme.css";`;
  const requests: string[] = [];
  const output = await rewriteCss(
    input,
    "https://example.test/assets/main.css",
    async (url) => {
      requests.push(url);
      return url.endsWith("theme.css") ? "theme-local.css" : "picture.svg";
    },
  );
  expect(requests.sort()).toEqual([
    "https://example.test/assets/real%20(one).svg",
    "https://example.test/assets/theme.css",
  ]);
  expect(output).toContain(svg);
  expect(output).toContain("url(#local)");
  expect(output).toContain('url("picture.svg#shape")');
  expect(output).toContain('@import "theme-local.css"');
});

test("static snapshots show reveal animations and provide a script-free mobile menu", async ({
  page,
}) => {
  const html = `<html><head><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'"><style>.transition--slide{opacity:0;transform:translateY(20%)}.transition--slide[data-animation-state=active]{opacity:1;transform:translateY(0)}.block-header-layout-mobile__dropdown{display:none}</style></head><body><button class="block-header__hamburger-menu">Menu</button><div class="block-header-layout-mobile__dropdown"><a href="about.html">About</a></div><section class="transition transition--slide"><h1>Saved gallery</h1><div data-animation-role="image">Photograph</div></section><div class="video"><img src="https://i.ytimg.com/vi/abcdefghijk/hqdefault.jpg"><button class="video__play--youtube"></button></div></body></html>`;
  const doc = new DOMParser().parseFromString(
    html,
    "text/html",
  ) as unknown as Document;
  staticPresentation(doc);
  expect(
    doc
      .querySelector("[data-animation-role=image]")
      ?.classList.contains("loaded"),
  ).toBe(true);
  expect(doc.querySelector(".zim-online-video")?.getAttribute("href")).toBe(
    "https://youtu.be/abcdefghijk",
  );
  expect(doc.querySelector(".video__play--youtube")).toBeNull();
  const emptyStore = doc.createElement("div");
  emptyStore.className = "block-product-list";
  doc.body.append(emptyStore);
  staticPresentation(doc);
  expect(emptyStore.textContent).toContain("Store catalog not captured");
  await page.setViewportSize({ width: 390, height: 844 });
  await page.setContent(doc.documentElement.outerHTML);
  await expect(page.locator("section")).toHaveCSS("opacity", "1");
  await expect(
    page.getByRole("heading", { name: "Saved gallery" }),
  ).toBeVisible();
  await page.getByText("Menu", { exact: true }).click();
  await expect(page.locator("details")).toHaveAttribute("open", "");
  await expect(
    page.getByRole("link", { name: "About", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Watch video online ↗" }),
  ).toHaveAttribute("href", "https://youtu.be/abcdefghijk");
});
