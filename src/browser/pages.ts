import { linkedAsset } from "../capture";
import { prepareImages } from "./images";
import { staticPresentation } from "./snapshot";
import { rewriteCss } from "./css";
import { DOMParser } from "linkedom/worker";
import {
  escape,
  resolveUrl,
  type BrowserJob,
  type BrowserEntry,
} from "./model";

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
  const title = document
    .querySelector("title")
    ?.textContent?.trim()
    .toLowerCase();
  if (
    [
      "just a moment...",
      "just a moment…",
      "attention required! | cloudflare",
    ].includes(title || "") &&
    ["/cdn-cgi/challenge-platform/", "_cf_chl_opt", "cf-chl-widget"].some(
      (marker) => html.includes(marker),
    )
  )
    throw new Error(
      "The site returned a browser verification challenge instead of content. Retry later, or import a saved page.",
    );
  const base =
    resolveUrl(
      document.querySelector("base[href]")?.getAttribute("href") || source,
      source,
    ) || source;
  prepareImages(document);
  staticPresentation(document);
  clean(document);
  const links = Array.from(document.querySelectorAll("a[href]"), (a) =>
    resolveUrl(a.getAttribute("href")!, base),
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
  const base =
    resolveUrl(
      document.querySelector("base[href]")?.getAttribute("href") || entry.url,
      entry.url,
    ) || entry.url;
  prepareImages(document);
  staticPresentation(document);
  clean(document);
  const selected = new Map(
    job.entries.filter((e) => e.selected).map((e) => [e.url, `${e.id}.html`]),
  );
  for (const a of document.querySelectorAll("a[href]")) {
    const original = a.getAttribute("href")!;
    if (original.startsWith("#")) continue;
    const url = resolveUrl(original, base);
    if (url) {
      const hash = new URL(original, base).hash;
      const saved =
        job.config.include_assets && linkedAsset(url) ? await asset(url) : "";
      a.setAttribute("href", (selected.get(url) || saved || url) + hash);
      a.setAttribute("rel", "noopener noreferrer");
    } else a.removeAttribute("href");
  }
  const load = async (value: string) => {
    const url = resolveUrl(value, base);
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
      base,
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
