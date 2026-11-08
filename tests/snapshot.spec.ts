import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { DOMParser } from "linkedom/worker";
import { staticPresentation } from "../src/browser/snapshot";
import { rewriteCss } from "../src/browser/css";
import { prepareImages } from "../src/browser/images";
import { discover, rewritePage } from "../src/browser/pages";
import type { BrowserJob, BrowserEntry } from "../src/browser/model";

test("HTTP-200 verification pages are rejected without blocking ordinary articles", () => {
  expect(() =>
    discover(
      `<title>Just a moment...</title><script src="/cdn-cgi/challenge-platform/start"></script>`,
      "https://example.test/",
    ),
  ).toThrow("verification challenge");
  expect(
    discover(
      `<title>How CAPTCHAs work</title><p>An article about browser verification.</p>`,
      "https://example.test/",
    ).title,
  ).toBe("How CAPTCHAs work");
  const doc = new DOMParser().parseFromString(
    `<iframe src="https://www.youtube-nocookie.com/embed/abcdefghijk"></iframe><iframe src="/embed/abcdefghijk"></iframe>`,
    "text/html",
  ) as unknown as Document;
  staticPresentation(doc);
  expect(doc.querySelectorAll(".zim-online-video")).toHaveLength(1);
});

test("lazy responsive images and document bases survive browser capture", async () => {
  const html = `<html><head><base href="https://example.test/photos/"></head><body><a href="story">Story</a><img src="data:image/gif;base64,AA" data-src="portrait.jpg" loading="lazy"><picture><source srcset="small.webp 400w, medium.webp 1200w, huge.webp 3000w"><img src="data:image/gif;base64,AA"></picture><img srcset="a,b.jpg 800w, enormous.jpg 2400w"></body></html>`;
  const doc = new DOMParser().parseFromString(
    html,
    "text/html",
  ) as unknown as Document;
  prepareImages(doc);
  expect(
    Array.from(doc.querySelectorAll("img"), (e) => e.getAttribute("src")),
  ).toEqual(["portrait.jpg", "medium.webp", "a,b.jpg"]);
  expect(discover(html, "https://example.test/").links).toContain(
    "https://example.test/photos/story",
  );
  const requested: string[] = [];
  const saved = await rewritePage(
    html,
    { url: "https://example.test/" } as BrowserEntry,
    {
      config: { include_assets: true },
      entries: [],
      archive_template: "atlas",
    } as unknown as BrowserJob,
    async (url) => {
      requested.push(url);
      return `saved-${requested.length}.jpg`;
    },
  );
  expect(requested).toEqual([
    "https://example.test/photos/portrait.jpg",
    "https://example.test/photos/medium.webp",
    "https://example.test/photos/a,b.jpg",
  ]);
  expect(saved).not.toContain("<base");
  expect(saved).not.toContain("loading=");
  expect(saved).toContain('src="saved-1.jpg"');
});

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
  await page.addStyleTag({
    content: await readFile("crates/engine/assets/archive.css", "utf8"),
  });
  await expect(page.locator("section")).toHaveCSS("opacity", "1");
  await expect(
    page.getByRole("heading", { name: "Saved gallery" }),
  ).toBeVisible();
  // Readers may expand all article <details>; navigation must remain closed.
  await page.evaluate(() =>
    document.querySelectorAll("details").forEach((node) => (node.open = true)),
  );
  await expect(
    page.getByRole("navigation", { name: "Site pages" }),
  ).toBeHidden();
  await page.getByRole("checkbox", { name: "Site menu", exact: true }).check();
  await expect(
    page.getByRole("navigation", { name: "Site pages" }),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "About", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Watch video online ↗" }),
  ).toHaveAttribute("href", "https://youtu.be/abcdefghijk");
});

test("saved product and lazy images become visible without removing unrelated classes", async ({
  page,
}) => {
  const doc = new DOMParser().parseFromString(
    `<html><head><style>.ecommerce-product-image--loading,.lazyloading{visibility:hidden}.unrelated{display:none}.product-carousel__image-wrapper{height:0;overflow:hidden}</style></head><body><div class="product-carousel__image-wrapper"><template><img alt="Saved product" class="ecommerce-product-image ecommerce-product-image--loading" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='20' height='20'%3E%3Crect width='20' height='20' fill='red'/%3E%3C/svg%3E"></template></div><img alt="Lazy photo" class="photo lazyloading" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='20' height='20'/%3E"><div class="unrelated">Keep hidden</div></body></html>`,
    "text/html",
  ) as unknown as Document;
  staticPresentation(doc);
  await page.setContent(doc.documentElement.outerHTML);
  await page.addStyleTag({
    content: await readFile("crates/engine/assets/archive.css", "utf8"),
  });
  await expect(page.getByAltText("Saved product")).toBeVisible();
  expect(
    await page
      .getByAltText("Saved product")
      .evaluate((el) => el.parentElement!.getBoundingClientRect().height),
  ).toBeGreaterThan(0);
  await expect(page.getByAltText("Lazy photo")).toBeVisible();
  await expect(page.getByAltText("Lazy photo")).toHaveClass("photo lazyloaded");
  await expect(page.getByText("Keep hidden")).toBeHidden();
});

test("offline page uses a single picture URL and preserves separate font faces", async () => {
  const html = `<html><head><style>@media screen {@font-face{font-family:test;src:local("Test"),url('/test.ttf') format('truetype'),url('/test.woff2') format('woff2')}} @font-face{font-family:test;font-weight:700;src:url('/bold.woff2')}</style></head><body><picture><source srcset="/photo.webp?crop=a,b 1x, /photo-large.webp 2x"><img src="/photo.jpg" style="--photo:url('/photo.webp?crop=a,b');--retina:url('/photo-large.webp');background-image:url('/overlay.svg')"></picture></body></html>`;
  const requested = new Set<string>();
  const saved = await rewritePage(
    html,
    { url: "https://example.test/" } as BrowserEntry,
    {
      config: { include_assets: true },
      entries: [],
      resources: { "https://example.test/test.ttf": { done: true } },
      archive_template: "atlas",
    } as unknown as BrowserJob,
    async (url) => {
      const name = new URL(url).pathname.slice(1);
      requested.add(name);
      return `saved-${name}`;
    },
  );
  expect([...requested].sort()).toEqual([
    "bold.woff2",
    "overlay.svg",
    "photo.jpg",
    "test.ttf",
  ]);
  expect(saved).toContain("saved-photo.jpg");
  expect(saved).toContain('local("Test")');
  expect(saved).not.toContain("photo-large.webp");
});
