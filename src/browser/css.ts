import { generate, parse, walk, type CssNode } from "css-tree";
import { resolveUrl } from "./model";

export function freezeImageUrls(
  css: string,
  variants: Set<string>,
  chosen: string,
) {
  try {
    const tree = parse(css, {
      context: "declarationList",
      parseCustomProperty: true,
    });
    let changed = false;
    walk(tree, (node) => {
      if (
        node.type === "Url" &&
        variants.has(node.value) &&
        node.value !== chosen
      ) {
        node.value = chosen;
        changed = true;
      }
    });
    return changed ? generate(tree) : css;
  } catch {
    return css;
  }
}

function compactFontSources(
  css: string,
  base: string,
  saved: (url: string) => boolean,
) {
  const edits: { start: number; end: number; value: string }[] = [];
  walk(parse(css, { positions: true }), (node) => {
    if (node.type !== "Atrule" || node.name.toLowerCase() !== "font-face")
      return;
    node.block?.children.forEach((decl) => {
      if (
        decl.type !== "Declaration" ||
        decl.property.toLowerCase() !== "src" ||
        decl.value.type !== "Value" ||
        !decl.value.loc
      )
        return;
      const groups: CssNode[][] = [[]];
      for (const child of decl.value.children) {
        if (child.type === "Operator" && child.value === ",") groups.push([]);
        else groups.at(-1)!.push(child);
      }
      if (
        groups.some(
          (group) =>
            !group.length ||
            group.some(
              (item) =>
                item.type === "Function" && item.name.toLowerCase() === "tech",
            ) ||
            group.filter((item) => item.type === "Url").length > 1,
        )
      )
        return;
      const ranked = groups.flatMap((group, index) => {
        const urlNode = group.find((item) => item.type === "Url");
        const url =
          urlNode?.type === "Url" ? resolveUrl(urlNode.value, base) : null;
        if (!url) return [];
        const path = new URL(url).pathname.toLowerCase();
        const format = path.endsWith(".woff2")
          ? 0
          : path.endsWith(".woff")
            ? 1
            : /\.(ttf|otf)$/.test(path)
              ? 2
              : 3;
        return [{ index, format, cached: saved(url) }];
      });
      if (ranked.length < 2) return;
      ranked.sort(
        (a, b) =>
          Number(b.cached) - Number(a.cached) ||
          a.format - b.format ||
          a.index - b.index,
      );
      const kept = groups.filter(
        (_, index) =>
          index === ranked[0].index ||
          !ranked.some((entry) => entry.index === index),
      );
      edits.push({
        start: decl.value.loc.start.offset,
        end: decl.value.loc.end.offset,
        value: kept
          .map((group) =>
            css.slice(
              group[0].loc!.start.offset,
              group.at(-1)!.loc!.end.offset,
            ),
          )
          .join(", "),
      });
    });
  });
  for (const edit of edits.sort((a, b) => b.start - a.start)) {
    css = css.slice(0, edit.start) + edit.value + css.slice(edit.end);
  }
  return css;
}

export async function rewriteCss(
  css: string,
  base: string,
  asset: (url: string) => Promise<string>,
  context: "stylesheet" | "declarationList" = "stylesheet",
  saved: (url: string) => boolean = () => false,
) {
  const references: {
    start: number;
    end: number;
    value: string;
    import: boolean;
  }[] = [];
  const add = (node: CssNode, value: string, imported: boolean) => {
    if (node.loc)
      references.push({
        start: node.loc.start.offset,
        end: node.loc.end.offset,
        value,
        import: imported,
      });
  };
  try {
    if (context === "stylesheet") css = compactFontSources(css, base, saved);
    walk(
      parse(css, { context, positions: true, parseCustomProperty: true }),
      (node) => {
        if (node.type === "Url") add(node, node.value, false);
        if (
          node.type === "Atrule" &&
          node.name.toLowerCase() === "import" &&
          node.prelude?.type === "AtrulePrelude"
        ) {
          const first = node.prelude.children.first;
          if (first?.type === "String") add(first, first.value, true);
        }
      },
    );
  } catch {
    return css;
  }
  for (const reference of references.sort((a, b) => b.start - a.start)) {
    const { value } = reference;
    if (/^(?:data:|#)/i.test(value)) continue;
    const url = resolveUrl(value, base),
      saved = url ? await asset(url) : "";
    const fragment = saved ? new URL(value, base).hash : "";
    const encoded = JSON.stringify(saved + fragment);
    const replacement = reference.import ? encoded : `url(${encoded})`;
    css =
      css.slice(0, reference.start) + replacement + css.slice(reference.end);
  }
  return css;
}
