export function staticPresentation(document: Document) {
  for (const list of document.querySelectorAll(".block-product-list")) {
    if (list.textContent?.trim() || list.querySelector("a[href]")) continue;
    const note = document.createElement("aside");
    note.className = "zim-missing-content";
    const heading = document.createElement("h2");
    heading.textContent = "Store catalog not captured";
    const text = document.createElement("p");
    text.textContent =
      "This catalog loads its products with JavaScript. Product listings are not included in this offline capture. Use Original source to browse the live store.";
    note.append(heading, text);
    list.replaceChildren(note);
  }
  for (const video of document.querySelectorAll(".video")) {
    const button = video.querySelector(".video__play--youtube"),
      src = video.querySelector("img[src]")?.getAttribute("src");
    if (!button || !src) continue;
    try {
      const url = new URL(src),
        match = url.pathname.match(/^\/(?:vi|vi_webp)\/([\w-]{11})\//);
      if (!["i.ytimg.com", "img.youtube.com"].includes(url.hostname) || !match)
        continue;
      const link = document.createElement("a");
      link.className = "zim-online-video";
      link.href = `https://youtu.be/${match[1]}`;
      link.textContent = "Watch video online ↗";
      button.replaceWith(link);
    } catch {
      /* Keep unknown embeds unchanged. */
    }
  }
  for (const element of document.querySelectorAll(
    "[class],[data-animation-role],[data-aos]",
  )) {
    if (
      element.hasAttribute("data-animation-role") ||
      ["transition--fade", "transition--slide", "transition--scale"].some((c) =>
        element.classList.contains(c),
      )
    ) {
      element.setAttribute("data-animation-state", "active");
      if (element.getAttribute("data-animation-role") === "image")
        element.classList.add("loaded");
    }
    if (element.hasAttribute("data-aos")) element.classList.add("aos-animate");
  }
  const dropdown = document.querySelector(
    ".block-header-layout-mobile__dropdown",
  );
  const buttons = document.querySelectorAll(
    "button.block-header__hamburger-menu",
  );
  if (!dropdown || !buttons.length) return;
  const links: HTMLAnchorElement[] = [],
    seen = new Set<string>();
  for (const a of dropdown.querySelectorAll("a[href]")) {
    const text = a.textContent?.replace(/\s+/g, " ").trim(),
      href = a.getAttribute("href")!;
    if (!text || seen.has(href + "\n" + text)) continue;
    seen.add(href + "\n" + text);
    const link = document.createElement("a");
    link.textContent = text;
    link.setAttribute("href", href);
    links.push(link);
    if (links.length === 100) break;
  }
  if (!links.length) return;
  for (const button of buttons) {
    const details = document.createElement("details");
    details.className = "zim-site-menu";
    const summary = document.createElement("summary");
    summary.textContent = "Menu";
    const nav = document.createElement("nav");
    nav.setAttribute("aria-label", "Site pages");
    links.forEach((link) => nav.append(link.cloneNode(true)));
    details.append(summary, nav);
    button.replaceWith(details);
  }
  document
    .querySelectorAll(".block-header-layout-mobile__dropdown")
    .forEach((menu) => menu.remove());
}
