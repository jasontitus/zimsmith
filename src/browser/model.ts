import { validateCapture } from "../capture";
import type { Config, Entry, Job } from "../types";

export interface Receipt {
  file: string;
  mime: string;
  size: number;
  etag?: string;
  last_modified?: string;
  revalidate?: boolean;
  total?: number;
  done: boolean;
}
export interface BrowserEntry extends Entry {
  text: string;
  file?: string;
  media?: string;
  mime?: string;
}
export interface BrowserJob extends Omit<Job, "entries"> {
  schema: 1;
  entries: BrowserEntry[];
  queue: { url: string; depth: number }[];
  visited: string[];
  resources: Record<string, Receipt>;
  output_file?: string;
  omissions?: Record<string, string>;
  refresh?:
    | (NonNullable<Job["refresh"]> & {
        previous: BrowserEntry[];
        fingerprints: Record<string, string>;
      })
    | null;
}
export const now = () => Math.floor(Date.now() / 1000);
export const defaults = {
  auto_start_small_jobs: true,
  auto_start_item_limit: 100,
  archive_template: "atlas" as const,
};
export function validate(config: Config) {
  validateCapture(config);
  const url = new URL(config.url);
  if (
    !["http:", "https:"].includes(url.protocol) ||
    url.username ||
    url.password
  )
    throw new Error(
      "Use a public HTTP or HTTPS URL without embedded credentials.",
    );
  for (const [n, min, max, name] of [
    [config.max_items, 1, 5000, "Item limit"],
    [config.max_depth, 0, 20, "Crawl depth"],
    [config.delay_ms, 1000, 60000, "Request spacing"],
    [config.max_resource_mb, 1, 1024, "File limit"],
  ] as const) {
    if (!Number.isInteger(n) || n < min || n > max)
      throw new Error(`${name} must be ${min}–${max}.`);
  }
  if (!config.title.trim() || config.title.length > 120)
    throw new Error("Give this archive a title of 1–120 characters.");
  if (
    !config.description.trim() ||
    config.description.length > 80 ||
    !config.creator.trim()
  )
    throw new Error("Provide a creator and a description of 1–80 characters.");
  if (!/^[a-z]{3}$/.test(config.language))
    throw new Error("Use a three-letter language code, such as eng.");
  // The native extractor depends on native TLS/QuickJS. Do not silently turn a
  // YouTube collection into an empty HTML crawl or pretend it is a direct file.
  if (
    config.kind === "media" &&
    !/\.(mp4|webm|mov|mp3|m4a|ogg|wav)(?:$|\?)/i.test(config.url)
  )
    throw new Error(
      "This browser build supports direct video/audio file URLs. YouTube and other collection extractors are available in the native Zimsmith app; they have not been ported to the browser engine yet.",
    );
}
export function inScope(candidate: string, source: string) {
  const url = new URL(candidate),
    root = new URL(source);
  return (
    url.origin === root.origin &&
    (root.pathname === "/" ||
      url.pathname === root.pathname ||
      url.pathname.startsWith(`${root.pathname.replace(/\/$/, "")}/`))
  );
}
export function resolveUrl(value: string, source: string): string | null {
  try {
    const url = new URL(value, source);
    if (
      !["http:", "https:"].includes(url.protocol) ||
      url.username ||
      url.password
    )
      return null;
    url.hash = "";
    return url.href;
  } catch {
    return null;
  }
}
export const escape = (s: string) =>
  s.replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ]!,
  );
