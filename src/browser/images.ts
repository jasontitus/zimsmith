import { freezeImageUrls } from "./css";

function candidates(value: string) {
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
  return candidates;
}

function fromSrcset(value: string): string | undefined {
  const options = candidates(value).sort((a, b) => a.score - b.score);
  return options.filter((c) => c.score <= 1600).at(-1)?.url || options[0]?.url;
}
export function prepareImages(document: Document) {
  for (const img of document.querySelectorAll("img")) {
    const attr = (name: string) => img.getAttribute(name)?.trim() || undefined;
    const picture = img.parentElement?.matches("picture")
      ? img.parentElement
      : null;
    const sets = Array.from(picture?.querySelectorAll("source") || []).flatMap(
      (element) => [
        element.getAttribute("data-srcset"),
        element.getAttribute("srcset"),
      ],
    );
    let source =
      ["data-src", "data-original", "data-lazy-src"].map(attr).find(Boolean) ||
      fromSrcset(attr("data-srcset") || attr("srcset") || "");
    if (
      !source &&
      (!attr("src") || attr("src")!.startsWith("data:")) &&
      picture
    ) {
      for (const set of sets) {
        source = fromSrcset(set || "");
        if (source) break;
      }
    }
    if (source) img.setAttribute("src", source);
    const chosen = attr("src"),
      style = attr("style");
    if (chosen && style && !chosen.startsWith("data:")) {
      const variants = new Set(
        [...sets, attr("srcset"), attr("data-srcset")]
          .flatMap((set) => candidates(set || ""))
          .map(({ url }) => url),
      );
      if (variants.size)
        img.setAttribute("style", freezeImageUrls(style, variants, chosen));
    }
    img.removeAttribute("loading");
  }
  document
    .querySelectorAll("picture > source")
    .forEach((source) => source.remove());
}
