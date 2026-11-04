import type { CaptureOptions, Config } from "./types";

export const captureDefaults: CaptureOptions = {
  auto_render_javascript: true,
  render_javascript: false,
  render_timeout_secs: 45,
  embedded_videos: true,
  discover_sitemaps: false,
  include_urls: [],
  seed_urls: [],
  exclude_urls: [],
  images: true,
  styles: true,
  fonts: true,
  media: true,
  documents: true,
  max_download_mb: null,
  video_quality: "balanced",
  subtitles: true,
  subtitle_language: "original",
  auto_captions: true,
};
export function captureOptions(config: Config): CaptureOptions {
  return { ...captureDefaults, ...config.capture };
}
export function normalizedCapture(config: Config): CaptureOptions {
  const value = captureOptions(config);
  return {
    ...value,
    include_urls: value.include_urls.map((s) => s.trim()).filter(Boolean),
    exclude_urls: value.exclude_urls.map((s) => s.trim()).filter(Boolean),
    subtitle_language: value.subtitle_language.trim(),
    render_javascript: config.kind === "website" && value.render_javascript,
    auto_render_javascript:
      config.kind === "website" && value.auto_render_javascript,
  };
}
export function matchesRule(pattern: string, value: string) {
  const url = new URL(value);
  const expression = pattern
    .replace(/[.+^${}()|[\]\\]/g, "\\$&")
    .replace(/\*/g, ".*")
    .replace(/\?/g, ".");
  return new RegExp(`^${expression}$`, "u").test(
    pattern.startsWith("/") ? url.pathname : url.href,
  );
}
export function pageAllowed(config: Config, url: string) {
  const c = captureOptions(config);
  return (
    !c.exclude_urls.some((p) => matchesRule(p, url)) &&
    (!c.include_urls.length || c.include_urls.some((p) => matchesRule(p, url)))
  );
}
export function assetAllowed(config: Config, url: string, mime = "") {
  const c = captureOptions(config),
    u = new URL(url);
  if (c.exclude_urls.some((p) => matchesRule(p, url))) return false;
  const ext = u.pathname.toLowerCase().split(".").pop() || "";
  if (
    /^image\//i.test(mime) ||
    /^(png|jpg|jpeg|gif|svg|webp|avif|ico|bmp)$/.test(ext)
  )
    return c.images;
  if (/^text\/css/i.test(mime) || ext === "css") return c.styles;
  if (/^font\//i.test(mime) || /^(woff2?|ttf|otf|eot)$/.test(ext))
    return c.fonts;
  if (
    /^(audio|video)\//i.test(mime) ||
    /^(mp4|webm|mp3|m4a|ogg|wav|mov|m3u8)$/.test(ext)
  )
    return c.media;
  if (
    /^application\/(pdf|epub\+zip|vnd.openxmlformats)/i.test(mime) ||
    /^(zip|zim|pdf|epub|docx?|xlsx?|pptx?)$/.test(ext)
  )
    return c.documents;
  return true;
}
export function validateCapture(config: Config) {
  const c = captureOptions(config);
  if (
    !Number.isInteger(c.render_timeout_secs) ||
    c.render_timeout_secs < 10 ||
    c.render_timeout_secs > 180
  )
    throw new Error("Browser rendering timeout must be 10–180 seconds.");
  if (c.render_javascript && c.max_download_mb !== null)
    throw new Error(
      "Browser rendering cannot enforce a strict download budget. Disable rendering or clear the download budget.",
    );
  if (c.seed_urls.length > 1000)
    throw new Error("Use at most 1000 additional starting URLs.");
  for (const source of c.seed_urls) {
    const url = new URL(source);
    if (
      !["https:", "http:"].includes(url.protocol) ||
      url.username ||
      url.password ||
      c.exclude_urls.some((p) => matchesRule(p, source))
    )
      throw new Error("A starting URL is invalid or excluded.");
  }
  if (config.kind !== "website" && c.seed_urls.length)
    throw new Error("URL lists support websites; use a playlist for media.");
  if (
    c.include_urls.length > 50 ||
    c.exclude_urls.length > 50 ||
    [...c.include_urls, ...c.exclude_urls].some(
      (r) => !r.trim() || r.length > 512,
    )
  )
    throw new Error("Use at most 50 rules of 1–512 characters each.");
  if (c.exclude_urls.some((p) => matchesRule(p, config.url)))
    throw new Error("The source URL matches an exclude rule.");
  if (
    c.max_download_mb !== null &&
    (!Number.isInteger(c.max_download_mb) ||
      c.max_download_mb < 1 ||
      c.max_download_mb > 1048576)
  )
    throw new Error("Download budget must be 1–1048576 MB, or blank.");
  if (
    !/^(original|[a-zA-Z0-9]+(?:-[a-zA-Z0-9]+)*)$/.test(c.subtitle_language) ||
    c.subtitle_language.length > 35
  )
    throw new Error(
      "Use original or a subtitle language code such as en or pt-BR.",
    );
}

export function linkedAsset(value: string) {
  return /\.(png|jpe?g|webp|gif|svg|avif|ico|mp4|webm|mov|mp3|m4a|ogg|wav|pdf|epub|docx?|xlsx?|pptx?|zip|zim|css|woff2?|ttf|otf)$/i.test(
    new URL(value).pathname,
  );
}
