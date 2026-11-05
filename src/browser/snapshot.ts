export function staticPresentation(document: Document) {
  for (const template of document.querySelectorAll<HTMLTemplateElement>(
    ".product-carousel__image-wrapper template",
  )) {
    template.replaceWith(template.content.cloneNode(true));
  }
  for (const wrapper of document.querySelectorAll(
    ".product-carousel__image-wrapper",
  )) {
    if (wrapper.querySelector("img"))
      wrapper.classList.add("zim-static-product-image");
  }
  for (const image of document.querySelectorAll("img")) {
    const lazy =
      image.classList.contains("lazyload") ||
      image.classList.contains("lazyloading");
    image.classList.remove(
      "ecommerce-product-image--loading",
      "lazyload",
      "lazyloading",
    );
    if (lazy) image.classList.add("lazyloaded");
  }
  for (const frame of document.querySelectorAll("iframe,embed,object")) {
    const value =
      frame.getAttribute("src") ||
      frame.getAttribute("data-src") ||
      frame.getAttribute("data");
    if (!value || !/^(https?:)?\/\//i.test(value)) continue;
    try {
      const url = new URL(value, "https://www.youtube.com/"),
        match = url.pathname.match(/^\/embed\/([\w-]{11})$/);
      if (
        ![
          "youtube.com",
          "www.youtube.com",
          "m.youtube.com",
          "youtube-nocookie.com",
          "www.youtube-nocookie.com",
        ].includes(url.hostname) ||
        !match
      )
        continue;
      const link = document.createElement("a");
      link.className = "zim-online-video";
      link.href = `https://www.youtube.com/watch?v=${match[1]}`;
      link.textContent = "Watch video online ↗ · Video not saved";
      frame.replaceWith(link);
    } catch {
      /* Unknown frames are removed by the capture sanitizer. */
    }
  }
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
  let index = 0;
  for (const button of buttons) {
    const menu = document.createElement("div");
    menu.className = "zim-site-menu";
    const toggle = document.createElement("input");
    toggle.type = "checkbox";
    toggle.className = "zim-menu-toggle";
    toggle.id = `zim-site-menu-toggle-${index++}`;
    toggle.setAttribute("aria-label", "Site menu");
    const label = document.createElement("label");
    label.setAttribute("for", toggle.id);
    label.textContent = "Menu";
    const nav = document.createElement("nav");
    nav.setAttribute("aria-label", "Site pages");
    links.forEach((link) => nav.append(link.cloneNode(true)));
    menu.append(toggle, label, nav);
    button.replaceWith(menu);
  }
  document
    .querySelectorAll(".block-header-layout-mobile__dropdown")
    .forEach((menu) => menu.remove());
}
