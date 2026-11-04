import { parse, walk, type CssNode } from "css-tree";
import { resolveUrl } from "./model";

export async function rewriteCss(
  css: string,
  base: string,
  asset: (url: string) => Promise<string>,
  context: "stylesheet" | "declarationList" = "stylesheet",
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
