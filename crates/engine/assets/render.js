(() => {
  if (window !== window.top || window.__zimsmithCapture) return;
  const options = window.__zimsmithRenderOptions || {};
  const started = performance.now();
  const deadline = Math.max(10, Math.min(180, options.timeout_secs || 45)) * 1000;
  const cap = Math.min(8 * 1024 * 1024, options.max_html_bytes || 8 * 1024 * 1024);
  const spacing = Math.max(1000, Math.min(86400000, options.delay_ms || 1000));
  const origins = new Map(), xhrs = new WeakMap();
  const catalogs = new Map();
  const state = { pending: 0, changed: started, scrolled: started, steps: 0, clicks: 0, height: 0, settled: false, done: false, rate_limit: null, warnings: [], result: null };
  const warn = message => { if (state.warnings.length < 20 && !state.warnings.includes(message)) state.warnings.push(message); };
  function limited(url, status, retry_after) {
    if (status === 429 || status === 503) state.rate_limit = { url: String(url), status, retry_after: retry_after || null };
  }
  async function gap(value) {
    let origin;
    try { origin = new URL(value, document.baseURI).origin; } catch { return; }
    const now = performance.now(), next = Math.max(now, origins.get(origin) || 0);
    origins.set(origin, next + spacing);
    if (next > now) await new Promise(resolve => setTimeout(resolve, Math.min(next - now, deadline)));
    if (state.done || state.rate_limit || performance.now() - started > deadline) throw new Error("Capture stopped");
  }
  const fetchOriginal = window.fetch.bind(window);
  window.fetch = async function(input, init) {
    const url = input instanceof Request ? input.url : String(input);
    state.pending++;
    try {
      await gap(url);
      const response = await fetchOriginal(input, init);
      limited(response.url || url, response.status, response.headers.get("retry-after"));
      return response;
    } finally { state.pending--; state.changed = performance.now(); }
  };
  const openOriginal = XMLHttpRequest.prototype.open, sendOriginal = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.open = function(method, url, ...args) {
    xhrs.set(this, { url: new URL(String(url), document.baseURI).href, async: args[0] !== false });
    return openOriginal.call(this, method, url, ...args);
  };
  XMLHttpRequest.prototype.send = function(body) {
    const request = this, info = xhrs.get(request);
    if (!info || !info.async) { warn("A synchronous page request was blocked because it cannot be paced."); throw new Error("Synchronous requests are unavailable during capture"); }
    state.pending++;
    let completed = false;
    const finish = () => {
      if (completed) return; completed = true; state.pending--; state.changed = performance.now();
      try { limited(request.responseURL || info.url, request.status, request.getResponseHeader("retry-after")); } catch {}
    };
    request.addEventListener("loadend", finish, { once: true });
    gap(info.url).then(() => { if (request.readyState === 1) sendOriginal.call(request, body); else finish(); }).catch(() => { request.abort(); finish(); });
  };
  // Render public content without opening dialogs, granting permissions, or playing media.
  window.open = () => null;
  window.alert = () => {};
  window.confirm = () => false;
  window.prompt = () => null;
  if (window.HTMLMediaElement) HTMLMediaElement.prototype.play = () => Promise.reject(new Error("Media downloads wait for approval"));
  document.addEventListener("submit", event => event.preventDefault(), true);
  const observer = new MutationObserver(() => { state.changed = performance.now(); });
  observer.observe(document, { childList: true, subtree: true, attributes: true, characterData: true });
  function copyImages(original, clone) {
    const sources = original.querySelectorAll("img"), targets = clone.querySelectorAll("img");
    sources.forEach((image, i) => {
      let src = image.currentSrc || image.src;
      if (!src || src.startsWith("data:")) src = image.getAttribute("data-src") || image.getAttribute("data-original") || src;
      if (src && !src.startsWith("blob:")) targets[i].setAttribute("src", new URL(src, document.baseURI).href);
      else if (src?.startsWith("blob:")) warn("A blob-backed image could not be saved automatically.");
      for (const name of ["srcset", "data-srcset", "data-src", "data-original", "data-lazy-src", "loading"]) targets[i].removeAttribute(name);
    });
  }
  // A small adapter layer: platform-specific catalogs plus accessible, button-paginated lists.
  // Ordinary pagination links already enter crawl discovery without clicking.
  function collectionDescriptors() {
    const found = [];
    for (const root of document.querySelectorAll(".block-product-list[id]")) {
      const count = root.querySelector(".product-list__total-count")?.textContent.match(/[\d,]+/);
      const content = root.querySelector(".product-list__content");
      if (content) found.push({ key: "catalog-" + root.id, root, content, items: Array.from(content.querySelectorAll(":scope > a.product-list__link")), next: root.querySelector('.product-list-pagination button[data-qa="button-next"]'), expected: count ? Number(count[0].replace(/,/g, "")) : 0, kind: "catalog" });
    }
    if (!/\/(?:checkout|cart|basket|billing|login|signin|oauth|payment)(?:[/?]|$)/i.test(location.pathname)) {
      let index = 0;
      for (const content of document.querySelectorAll('main [role="list"], [role="main"] [role="list"]')) {
        if (content.closest(".block-product-list,form")) continue;
        const parent = content.closest('main,[role="main"]');
        const nav = Array.from(parent.querySelectorAll('nav[aria-label],[role="navigation"][aria-label]')).find(node => /paginat|page.*nav|nav.*page/i.test(node.getAttribute("aria-label")));
        if (!nav || nav.closest("form")) continue;
        const next = Array.from(nav.querySelectorAll("button")).find(button => /^(?:go to )?next(?: page)?$/i.test(button.getAttribute("aria-label") || button.textContent.trim()));
        if (!next || (parent.querySelectorAll('[role="list"]').length > 1 && (!content.id || next.getAttribute("aria-controls") !== content.id))) continue;
        const items = Array.from(content.children).filter(node => node.getAttribute("role") === "listitem" || node.tagName === "LI");
        const expected = Number(content.getAttribute("aria-setsize") || items[0]?.getAttribute("aria-setsize") || 0);
        const key = "list-" + index++;
        if (nav.getAttribute("data-zim-pagination") !== key) nav.setAttribute("data-zim-pagination", key);
        found.push({ key, root: content, content, items, next, expected: expected > 0 ? expected : 0, kind: "list" });
      }
    }
    return found;
  }
  function collectCatalogs(now) {
    for (const descriptor of collectionDescriptors()) {
      const { key, root, items, next, expected, kind } = descriptor;
      if (!items.length) continue;
      if (root.getAttribute("data-zim-collection") !== key) root.setAttribute("data-zim-collection", key);
      let catalog = catalogs.get(key);
      if (!catalog) { catalog = { saved: new Map(), pages: 0, signature: "", waiting: 0, changed: now, stalled: false, expected: 0, url: location.href, kind, finished: false, template: null }; catalogs.set(key, catalog); }
      catalog.expected = expected;
      const itemKey = node => (node.matches("a[href]") ? node.href : node.querySelector("a[href]")?.href) || node.id || node.getAttribute("data-id") || node.textContent.trim().slice(0, 500);
      const signature = items.map(itemKey).join("\n");
      if (signature !== catalog.signature) {
        catalog.signature = signature; catalog.pages++; catalog.waiting = 0; catalog.changed = now;
        catalog.template = root.cloneNode(true);
        for (const item of items) {
          if (catalog.saved.size >= 500) break;
          const copy = item.cloneNode(true);
          if (item.matches("a[href]")) copy.setAttribute("href", item.href);
          for (const link of copy.querySelectorAll("a[href]")) link.setAttribute("href", new URL(link.getAttribute("href"), document.baseURI).href);
          copyImages(item, copy); catalog.saved.set(itemKey(item), copy);
        }
      }
      catalog.finished = (!next || next.disabled || next.getAttribute("aria-disabled") === "true") || (expected > 0 && catalog.saved.size >= expected);
      if (catalog.waiting) {
        if (now - catalog.waiting < 5000) return true;
        catalog.stalled = true; warn("List pagination did not advance; only saved items are included.");
      }
      // Image fades, resize observers and unrelated animations must not hold pagination forever.
      if (!catalog.finished && !catalog.stalled && catalog.pages < 20 && catalog.saved.size < 500 && now - catalog.changed >= 500 && !next.closest("form")) {
        catalog.waiting = now; next.click(); state.changed = now; return true;
      }
    }
    return false;
  }
  function snapshot(settled) {
    state.done = true; state.settled = settled; observer.disconnect();
    const clone = document.documentElement.cloneNode(true);
    clone.querySelectorAll("script,noscript").forEach(node => node.remove());
    copyImages(document.documentElement, clone);
    let catalogUrl;
    for (const [id, catalog] of catalogs) {
      let list = clone.querySelector(`[data-zim-collection="${CSS.escape(id)}"]`);
      let content = catalog.kind === "catalog" ? list?.querySelector(".product-list__content") : list;
      // Frameworks replace the whole list with a skeleton while fetching the next page.
      // Keep captured cards when a deadline falls inside that transition.
      if (list && !content && catalog.template) {
        const restored = catalog.template.cloneNode(true); list.replaceWith(restored); list = restored;
        content = catalog.kind === "catalog" ? list.querySelector(".product-list__content") : list;
      }
      if (!content) { settled = false; warn("A captured list disappeared before its snapshot could be saved."); continue; }
      content.replaceChildren(...Array.from(catalog.saved.values(), node => node.cloneNode(true)));
      list.querySelectorAll(".product-list-pagination").forEach(node => node.remove());
      clone.querySelectorAll(`[data-zim-pagination="${CSS.escape(id)}"]`).forEach(node => node.remove());
      list.querySelectorAll("[data-qa=product-sort-dropdown]").forEach(node => { node.setAttribute("disabled", ""); node.setAttribute("title", "Showing the captured catalog order"); });
      list.querySelectorAll("[data-qa=productlistsection-btn-addtobag]").forEach(node => node.remove());
      const count = list.querySelector(".product-list__total-count");
      if (count) count.textContent = `${catalog.saved.size} products captured`;
      if (catalog.expected > catalog.saved.size || !catalog.finished) { settled = false; warn(`List capture is incomplete: ${catalog.saved.size}${catalog.expected ? " of " + catalog.expected : ""} items were saved.`); }
      if (!catalogUrl) catalogUrl = catalog.url;
    }
    clone.querySelectorAll("script,noscript").forEach(node => node.remove());
    for (const video of clone.querySelectorAll("video,audio")) { video.removeAttribute("autoplay"); video.setAttribute("preload", "none"); }
    // Frameworks may populate CSSOM rules without writing text into their style nodes.
    let css = "";
    for (const sheet of Array.from(document.styleSheets).slice(0, 200)) {
      try { if (!sheet.href && sheet.ownerNode?.tagName === "STYLE") css += Array.from(sheet.cssRules, rule => rule.cssText).join("\n"); } catch {}
      if (css.length > 1024 * 1024) { warn("Runtime styles reached the 1 MB capture limit."); css = css.slice(0, 1024 * 1024); break; }
    }
    if (css) { const style = document.createElement("style"); style.textContent = css; clone.querySelector("head")?.append(style); }
    const base = document.createElement("base"); base.href = document.baseURI; clone.querySelector("head")?.prepend(base);
    const html = "<!doctype html>" + clone.outerHTML;
    if (new TextEncoder().encode(html).length > cap) throw new Error("Rendered HTML exceeds the 8 MB capture limit");
    const resources = performance.getEntriesByType("resource");
    state.result = { url: catalogUrl || location.href, html, settled, resource_count: resources.length, known_transfer_bytes: resources.reduce((total, resource) => total + (resource.transferSize || 0), 0), warnings: state.warnings, rate_limit: state.rate_limit };
    if (!settled && !state.rate_limit) warn("The page reached its rendering time/scroll limit; some dynamic content may be missing.");
    return state.result;
  }
  function pollContent() {
    if (state.result) return state.result;
    if (/^(just a moment(?:\.\.\.|…)|attention required! \| cloudflare)$/i.test(document.title.trim()) && (document.querySelector('script[src*="/cdn-cgi/challenge-platform/"]') || document.documentElement.outerHTML.includes("_cf_chl_opt"))) throw new Error("The browser received a verification challenge instead of page content");
    if (state.rate_limit) return snapshot(false);
    const now = performance.now(), elapsed = now - started;
    if (elapsed >= deadline) return snapshot(false);
    for (const media of document.querySelectorAll("video,audio")) { media.pause(); if (media.preload !== "none") media.preload = "none"; }
    if (document.readyState !== "complete" || state.pending) return null;
    if (collectCatalogs(now)) return null;
    if (catalogs.size && Array.from(catalogs.values()).every(catalog => catalog.finished && !catalog.waiting && now - catalog.changed >= 1000) && elapsed >= 3000) return snapshot(true);
    const height = Math.max(document.body?.scrollHeight || 0, document.documentElement.scrollHeight);
    if (now - state.scrolled >= 750 && state.steps < 20) {
      const bottom = window.scrollY + window.innerHeight >= height - 4;
      if (!bottom || height !== state.height) {
        window.scrollTo(0, Math.min(height, window.scrollY + Math.max(500, window.innerHeight * .85)));
        state.scrolled = now; state.height = height; state.steps++; state.changed = now;
        return null;
      }
      if (state.clicks < 5) {
        const more = Array.from(document.querySelectorAll("button")).find(button => !button.disabled && button.getClientRects().length && /^(load more|show more|more products|more posts)(?:\s*\(\d+\))?$/i.test(button.textContent.trim()));
        if (more) { more.click(); state.clicks++; state.scrolled = now; state.changed = now; return null; }
      }
    }
    if (elapsed >= 3000 && now - state.changed >= 2000) return snapshot(state.steps < 20);
    return null;
  }
  function poll() {
    const result = pollContent();
    if (result) return result;
    const lists = Array.from(catalogs.values());
    const captured_items = lists.reduce((n, list) => n + list.saved.size, 0);
    const expected_items = lists.reduce((n, list) => n + list.expected, 0);
    return { progress: { captured_items, expected_items: expected_items || null, pages: lists.reduce((n, list) => n + list.pages, 0), pending_requests: state.pending, elapsed_secs: Math.floor((performance.now() - started) / 1000) } };
  }
  Object.defineProperty(window, "__zimsmithCapture", { value: Object.freeze({ poll }), configurable: false });
})();
