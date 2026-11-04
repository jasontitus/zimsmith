function fromSrcset(value: string): string | undefined {
  let rest = value.trim();
  const candidates: { url: string; score: number }[] = [];
  while (rest && candidates.length < 100) {
    const end = rest.search(/\s/),
      token = end < 0 ? rest : rest.slice(0, end);
    rest = end < 0 ? "" : rest.slice(end).trimStart();
    let url = token,
      descriptor = "";
    if (token.endsWith(",")) url = token.replace(/,+$/, "");
    else {
      const comma = rest.indexOf(",");
      descriptor = (comma < 0 ? rest : rest.slice(0, comma)).trim();
      rest = comma < 0 ? "" : rest.slice(comma + 1).trimStart();
    }
    if (!url || url.startsWith("data:")) continue;
    const score = /^\d+(?:\.\d+)?w$/.test(descriptor)
      ? parseFloat(descriptor)
      : /^\d+(?:\.\d+)?x$/.test(descriptor)
        ? parseFloat(descriptor) * 800
        : 800;
    if (Number.isFinite(score) && score > 0) candidates.push({ url, score });
  }
  candidates.sort((a, b) => a.score - b.score);
  return (
    candidates.filter((c) => c.score <= 1600).at(-1)?.url || candidates[0]?.url
  );
}
export function prepareImages(document: Document) {
  for (const img of document.querySelectorAll("img")) {
    const attr = (name: string) => img.getAttribute(name)?.trim() || undefined;
    let source =
      ["data-src", "data-original", "data-lazy-src"].map(attr).find(Boolean) ||
      fromSrcset(attr("data-srcset") || attr("srcset") || "");
    if (
      !source &&
      (!attr("src") || attr("src")!.startsWith("data:")) &&
      img.parentElement?.tagName.toLowerCase() === "picture"
    ) {
      for (const candidate of img.parentElement.querySelectorAll("source")) {
        source = fromSrcset(
          candidate.getAttribute("data-srcset") ||
            candidate.getAttribute("srcset") ||
            "",
        );
        if (source) break;
      }
    }
    if (source) img.setAttribute("src", source);
    img.removeAttribute("loading");
  }
}
