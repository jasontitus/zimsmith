import { linkedAsset } from "../capture";
import { staticPresentation } from "./snapshot";
export { rewriteCss } from "./css";
import { rewriteCss } from "./css";
import { DOMParser } from "linkedom/worker";
import templateData from "./templates.json";
import css from "../../crates/engine/assets/archive.css?raw";
import search from "../../crates/engine/assets/search.js?raw";
import mediaScript from "../../crates/engine/assets/media.js?raw";
import gallery from "../../crates/engine/assets/gallery.js?raw";
import sans from "../../crates/engine/assets/fonts/dm-sans.woff2?url";
import display from "../../crates/engine/assets/fonts/manrope.woff2?url";
import icon from "../../crates/engine/assets/zimsmith-icon-48.png?url";
import license from "../../crates/engine/assets/fonts/LICENSE.txt?raw";
import {
  escape,
  resolveUrl,
  type BrowserJob,
  type BrowserEntry,
} from "./model";
import type { ArchiveTemplate } from "../types";
import type { ZimItem } from "./zim";

export const templates = templateData.catalog;
export const parse = (html: string) =>
  new DOMParser().parseFromString(html, "text/html") as unknown as Document;
export function clean(document: Document) {
  document
    .querySelectorAll(
      "script,iframe,frame,frameset,object,embed,base,form,meta[http-equiv],link:not([rel~='stylesheet'])",
    )
    .forEach((e) => e.remove());
  for (const element of document.querySelectorAll("*"))
    for (const attribute of Array.from(element.attributes))
      if (
        /^on/i.test(attribute.name) ||
        [
          "srcdoc",
          "ping",
          "integrity",
          "nonce",
          "crossorigin",
          "autoplay",
        ].includes(attribute.name)
      )
        element.removeAttribute(attribute.name);
}
export function discover(html: string, source: string) {
  const document = parse(html);
  staticPresentation(document);
  clean(document);
  const links = Array.from(document.querySelectorAll("a[href]"), (a) =>
    resolveUrl(a.getAttribute("href")!, source),
  ).filter((s): s is string => !!s);
  return {
    title: (
      document.querySelector("title")?.textContent ||
      new URL(source).pathname ||
      source
    )
      .trim()
      .slice(0, 500),
    text: (document.body?.textContent || "")
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, 50000),
    links,
    embedded_videos: document.querySelectorAll(".zim-online-video").length,
    missing_catalogs: document.querySelectorAll(".zim-missing-content").length,
  };
}

export async function rewritePage(
  html: string,
  entry: BrowserEntry,
  job: BrowserJob,
  asset: (url: string) => Promise<string>,
) {
  const document = parse(html);
  staticPresentation(document);
  clean(document);
  const selected = new Map(
    job.entries.filter((e) => e.selected).map((e) => [e.url, `${e.id}.html`]),
  );
  for (const a of document.querySelectorAll("a[href]")) {
    const original = a.getAttribute("href")!;
    if (original.startsWith("#")) continue;
    const url = resolveUrl(original, entry.url);
    if (url) {
      const hash = new URL(original, entry.url).hash;
      const saved =
        job.config.include_assets && linkedAsset(url) ? await asset(url) : "";
      a.setAttribute("href", (selected.get(url) || saved || url) + hash);
      a.setAttribute("rel", "noopener noreferrer");
    } else a.removeAttribute("href");
  }
  const load = async (value: string) => {
    const url = resolveUrl(value, entry.url);
    return url && job.config.include_assets ? asset(url) : "";
  };
  for (const element of document.querySelectorAll(
    "[src], [poster], link[rel~='stylesheet']",
  )) {
    const attribute = element.matches("link")
      ? "href"
      : element.hasAttribute("src")
        ? "src"
        : "poster";
    const value = element.getAttribute(attribute);
    if (!value) continue;
    if (value.startsWith("data:") && element.matches("img")) continue;
    const saved = await load(value);
    if (saved) element.setAttribute(attribute, saved);
    else element.removeAttribute(attribute);
  }
  for (const element of document.querySelectorAll("[srcset]"))
    element.removeAttribute("srcset");
  for (const element of document.querySelectorAll("style,[style]")) {
    const value =
      element.tagName.toLowerCase() === "style"
        ? element.textContent || ""
        : element.getAttribute("style") || "";
    const rewritten = await rewriteCss(
      value,
      entry.url,
      load,
      element.tagName.toLowerCase() === "style"
        ? "stylesheet"
        : "declarationList",
    );
    if (element.tagName.toLowerCase() === "style")
      element.textContent = rewritten;
    else element.setAttribute("style", rewritten);
  }
  const meta = document.createElement("meta");
  meta.setAttribute("http-equiv", "Content-Security-Policy");
  meta.setAttribute(
    "content",
    "default-src 'none'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' data:; media-src 'self'; base-uri 'none'; form-action 'none'",
  );
  document.head.prepend(meta);
  const style = document.createElement("link");
  style.setAttribute("rel", "stylesheet");
  style.setAttribute("href", "studio.css");
  document.head.append(style);
  const nav = document.createElement("nav");
  nav.className = "zim-archive-nav";
  nav.setAttribute("data-zim-template", job.archive_template);
  nav.innerHTML = job.import
    ? `<a href="index.html">← Collection contents</a><span>Imported saved content</span>`
    : `<a href="index.html">← Collection contents</a><span>Saved for offline reading</span><a href="${escape(entry.url)}" rel="noopener noreferrer">Original source ↗</a>`;
  document.body.prepend(nav);
  return "<!doctype html>" + document.documentElement.outerHTML;
}
interface CatalogDocument {
  title: string;
  url: string;
  excerpt: string;
  kind: string;
  detail: string;
  thumbnail: null;
  media: string | null;
}
export function renderIndex(job: BrowserJob) {
  const documents: CatalogDocument[] = [],
    index: Record<string, number[]> = Object.create(null);
  let postings = 0;
  for (const entry of job.entries.filter((e) => e.selected)) {
    const id = documents.length,
      media = !!entry.media,
      audio = /^audio\//.test(entry.mime || "");
    documents.push({
      title: entry.title,
      url: `${entry.id}.html`,
      excerpt: entry.text.slice(0, 160) || "Saved for offline discovery.",
      kind: media ? (audio ? "Audio" : "Video") : "Article",
      detail: media
        ? "Offline playback"
        : `${Math.max(1, Math.ceil(entry.text.split(/\s+/).length / 220))} min read`,
      thumbnail: null,
      media: entry.media || null,
    });
    for (const token of new Set(
      `${entry.title} ${entry.text}`
        .toLowerCase()
        .split(/[^\p{L}\p{N}]+/u)
        .filter(Boolean),
    )) {
      (index[token] ||= []).push(id);
      if (++postings > 1000000)
        throw new Error(
          "The browser search index exceeds its memory budget. Split this collection into smaller archives.",
        );
    }
  }
  const html = templateData.shells[job.archive_template];
  const document = parse(html);
  document.title = job.config.title;
  document.documentElement.setAttribute("lang", job.config.language);
  document.getElementById("archive-title")!.textContent = job.config.title;
  document.querySelector(".archive-description")!.textContent =
    job.config.description;
  document.querySelector(".collection-meta")!.innerHTML =
    `<span>${documents.length} items</span><span aria-hidden="true">/</span><span>Curated by ${escape(job.config.creator)}</span>`;
  document
    .querySelector(".source-link")!
    .setAttribute("href", job.import ? "#collection" : job.config.url);
  if (job.import)
    document.querySelector(".source-link")!.textContent =
      "Imported saved files";
  const report = document.createElement("span");
  report.innerHTML = "<a href='capture-report.html'>Capture report</a>";
  document.querySelector(".collection-meta")!.append(report);
  if (job.capture_report?.omitted_assets) {
    const notice = document.createElement("aside");
    notice.className = "archive-noscript";
    notice.textContent = `${job.capture_report.omitted_assets} assets were omitted. See the capture report for details.`;
    document.querySelector(".archive-collection")!.before(notice);
  }
  document.getElementById("count")!.textContent =
    `${documents.length} items to explore`;
  const list = document.getElementById("contents")!;
  list.innerHTML = documents
    .map((item, n) => {
      const copy = `<span class="entry-copy"><span class="entry-meta"><span class="entry-kind">${item.kind}</span><span>${escape(item.detail)}</span></span><h3>${escape(item.title)}</h3><span class="entry-excerpt">${escape(item.excerpt)}</span><span class="entry-bottom"><span>${job.archive_template === "screening_room" ? "More info" : `Open ${item.kind.toLowerCase()}`}</span><span aria-hidden="true">↗</span></span></span>`;
      if (job.archive_template === "screening_room") {
        const art = `<span class="video-art artwork-${n % 4}"><span class="artwork-number" aria-hidden="true">${String(n + 1).padStart(2, "0")}</span><span class="video-play" aria-hidden="true">${item.media ? "▶" : "↗"}</span><span class="video-duration">${escape(item.detail)}</span></span>`;
        const thumbnail = item.media
          ? `<button type="button" class="video-thumbnail" data-play-src="${escape(item.media)}" data-play-kind="${item.kind}" data-play-title="${escape(item.title)}" aria-label="Play ${escape(item.title)}">${art}</button>`
          : `<a class="video-thumbnail" href="${item.url}" aria-label="Open ${escape(item.title)}">${art}</a>`;
        return `<li class="archive-item" data-document="${n}"><article class="gallery-card">${thumbnail}<a class="entry-detail" href="${item.url}">${copy}</a></article></li>`;
      }
      return `<li class="archive-item" data-document="${n}"><a class="entry-link" href="${item.url}"><span class="entry-number">${String(n + 1).padStart(2, "0")}</span>${copy}</a></li>`;
    })
    .join("");
  document.getElementById("catalog")!.textContent = JSON.stringify({
    documents,
    index,
  })
    .replace(/</g, "\\u003c")
    .replace(/>/g, "\\u003e")
    .replace(/&/g, "\\u0026");
  return "<!doctype html>" + document.documentElement.outerHTML;
}
export function renderMedia(job: BrowserJob, entry: BrowserEntry) {
  const tag = /^audio\//.test(entry.mime || "") ? "audio" : "video";
  return `<!doctype html><html lang="${escape(job.config.language)}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'self'; script-src 'self'; font-src 'self'; media-src 'self'; base-uri 'none'; form-action 'none'"><title>${escape(entry.title)}</title><link rel="stylesheet" href="studio.css"></head><body class="zim-archive" data-zim-template="${job.archive_template}"><nav class="zim-archive-nav"><a href="index.html">← Collection contents</a></nav><main class="archive-media"><h1>${escape(entry.title)}</h1><p>${escape(job.config.title)}</p><div class="media-player"><${tag} id="video-track" controls playsinline preload="metadata" src="${escape(entry.media!)}"></${tag}></div><section class="media-description"><h2>About this item</h2><p>${escape(entry.text)}</p></section><a class="source-link" href="${escape(job.import ? "index.html" : entry.url)}" rel="noopener noreferrer">${job.import ? "Imported saved files" : "Original source ↗"}</a></main><script src="media.js"></script></body></html>`;
}
let assets: Promise<ZimItem[]> | undefined;
export function bundledAssets() {
  return (assets ||= Promise.all(
    [
      { path: "studio.css", mime: "text/css", content: new Blob([css]) },
      {
        path: "media.js",
        mime: "text/javascript",
        content: new Blob([mediaScript]),
      },
      {
        path: "search.js",
        mime: "application/javascript",
        content: new Blob([search]),
      },
      {
        path: "gallery.js",
        mime: "application/javascript",
        content: new Blob([gallery]),
      },
      {
        path: "font-licenses.txt",
        mime: "text/plain",
        content: new Blob([license]),
      },
      ...[
        ["archive-sans.woff2", "font/woff2", sans],
        ["archive-display.woff2", "font/woff2", display],
        ["favicon.png", "image/png", icon],
      ].map(async ([path, mime, url]) => ({
        path,
        mime,
        content: await (await fetch(url)).blob(),
      })),
    ].map(async (item) => ({ ...(await item), title: (await item).path })),
  ));
}
export async function preview(template: ArchiveTemplate) {
  const job = {
    archive_template: template,
    config: {
      title: "A collection worth keeping",
      description: "Stories, ideas, and a little more discovery.",
      creator: "You",
      language: "eng",
      url: "https://example.com",
    },
    entries: [
      "A different view of the world",
      "Ideas for a curious afternoon",
      "Something worth coming back to",
    ].map((title, n) => ({
      id: `sample-${n}`,
      title,
      selected: true,
      text: "An example from your collection, ready to explore offline.",
    })),
  } as BrowserJob;
  let html = renderIndex(job);
  const bundled = await bundledAssets();
  let styles = css;
  for (const item of bundled.filter((i) => /woff2$/.test(i.path))) {
    const bytes = new Uint8Array(await item.content.arrayBuffer());
    let binary = "";
    for (const byte of bytes) binary += String.fromCharCode(byte);
    styles = styles.replaceAll(
      item.path,
      `data:font/woff2;base64,${btoa(binary)}`,
    );
  }
  const document = parse(html);
  document
    .querySelectorAll("script,link[rel='stylesheet']")
    .forEach((e) => e.remove());
  document
    .querySelector("meta[http-equiv]")!
    .setAttribute(
      "content",
      "default-src 'none'; style-src 'unsafe-inline'; font-src data:; img-src data:; base-uri 'none'; form-action 'none'",
    );
  const style = document.createElement("style");
  style.textContent = styles;
  document.head.append(style);
  document.querySelectorAll("img").forEach((img) => img.remove());
  return "<!doctype html>" + document.documentElement.outerHTML;
}
