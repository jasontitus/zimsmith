import type { CaptureSummary } from "../types";
import { discover } from "./content";
import { escape, type BrowserJob } from "./model";
import { readFile } from "./storage";
import { unavailableAsset } from "./network";
const label = (value: string) => {
  try {
    const u = new URL(value);
    return u.origin + u.pathname;
  } catch {
    return "Unavailable URL";
  }
};
export async function captureReport(
  job: BrowserJob,
  dir: FileSystemDirectoryHandle,
  paths: Map<string, string>,
) {
  const selected = job.entries.filter((e) => e.selected),
    urls = new Set(selected.map((e) => e.url)),
    online = new Set<string>();
  const omitted = Object.entries(job.omissions || {});
  let embeddedVideos = 0;
  const summary: CaptureSummary = {
    captured_items: selected.length,
    saved_assets: [...paths.values()].filter(Boolean).length,
    omitted_assets: omitted.length,
    excluded_assets: omitted.filter(([, reason]) => reason.includes("settings"))
      .length,
    failed_assets: omitted.filter(([, reason]) => unavailableAsset(reason))
      .length,
    online_links: 0,
    subtitle_tracks: 0,
    caption_gaps: 0,
    search_limited: false,
    warnings: job.logs
      .filter((log) => log.message.startsWith("WARC import omitted"))
      .map((log) => log.message)
      .slice(-10),
  };
  for (const entry of selected) {
    if (!entry.media && entry.file) {
      const page = discover(
        await (await readFile(dir, entry.file)).text(),
        entry.url,
      );
      embeddedVideos += page.embedded_videos;
      if (page.missing_catalogs)
        summary.warnings.push(
          `Client-rendered store catalog not captured: ${entry.title}. Product listings require the live site.`,
        );
      for (const url of page.links) {
        if (!urls.has(url) && !paths.get(url) && online.size < 5000)
          online.add(label(url));
      }
      if (
        summary.warnings.length < 30 &&
        (!entry.text.trim() ||
          /just a moment|access denied|captcha/i.test(entry.title))
      )
        summary.warnings.push(
          `Check this possibly empty or blocked capture: ${entry.title}`,
        );
    }
  }
  summary.online_links = online.size;
  if (embeddedVideos)
    summary.warnings.push(
      `${embeddedVideos} embedded YouTube videos are labeled as online links. Their thumbnails are saved; the movies were not downloaded in this website capture.`,
    );
  const counts = Object.entries({
    "Captured items": summary.captured_items,
    "Saved assets": summary.saved_assets,
    "Omitted assets": summary.omitted_assets,
    "Excluded by settings": summary.excluded_assets,
    "Online-only links": summary.online_links,
  })
    .map(
      ([name, count]) =>
        `<tr><th scope="row">${name}</th><td>${count}</td></tr>`,
    )
    .join("");
  const assets = omitted
    .map(
      ([url, reason]) => `<li>${escape(label(url))} — ${escape(reason)}</li>`,
    )
    .join("");
  const warnings = summary.warnings
    .map((w) => `<li>${escape(w)}</li>`)
    .join("");
  const links = [...online].map((url) => `<li>${escape(url)}</li>`).join("");
  const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'self'; font-src 'self'; img-src 'self'"><title>Capture report</title><link rel="stylesheet" href="studio.css"></head><body class="zim-archive"><main class="archive-media capture-report"><a href="index.html">Back to collection</a><h1>Capture report</h1><p>Saved content and omissions. Zimsmith verifies the finished file’s checksum and directory tables before marking it ready. This does not guarantee that every source page or media file displays correctly.</p><table>${counts}</table><ul>${warnings}</ul><h2>Omitted assets</h2><ul>${assets}</ul><h2>Links still outside this archive</h2><ul>${links}</ul></main></body></html>`;
  return {
    summary,
    html,
    json: JSON.stringify({
      summary,
      items: selected.map((e) => ({ title: e.title, url: label(e.url) })),
      omitted_assets: omitted.map(([url, reason]) => ({
        url: label(url),
        reason,
      })),
      online_links: [...online],
    }),
  };
}
