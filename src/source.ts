// Normalize only URLs entered as sources, never relative links found in a crawl.
export function normalizeSourceUrl(input: string): string {
  const value = input.trim();
  if (!value) return "";
  if (/[\p{Cc}]/u.test(value))
    throw new Error("The URL contains an unexpected control character");
  const authority = value.split(/[/?#]/, 1)[0];
  const hostPort = /^[a-zA-Z0-9._-]+:[0-9]+$/.test(authority);
  const scheme = /^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(value);
  if (scheme && !/^https?:/i.test(value) && !hostPort)
    throw new Error("Only HTTP and HTTPS URLs are supported");
  const candidate = value.startsWith("//")
    ? `https:${value}`
    : /^https?:/i.test(value) || value.includes("://")
      ? value
      : `https://${value}`;
  let url: URL;
  try {
    url = new URL(candidate);
  } catch {
    throw new Error(
      "Enter a website such as cnn.com or a complete HTTP/HTTPS URL",
    );
  }
  if (!["http:", "https:"].includes(url.protocol))
    throw new Error("Only HTTP and HTTPS URLs are supported");
  if (url.username || url.password)
    throw new Error("URLs containing credentials are not supported");
  url.hash = "";
  return url.href;
}

// Match the engine's native media URL detection. The engine also checks this
// on creation and when retrying jobs made by older versions of the app.
export function isNativeMediaUrl(value: string): boolean {
  try {
    const url = new URL(normalizeSourceUrl(value));
    if (!["http:", "https:"].includes(url.protocol)) return false;
    return (
      [
        "youtube.com",
        "www.youtube.com",
        "m.youtube.com",
        "music.youtube.com",
        "youtu.be",
      ].includes(url.hostname) ||
      /\.(mp4|webm|mp3|m4a|ogg|wav|mov)$/i.test(url.pathname)
    );
  } catch {
    return false;
  }
}
