import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { buildSync } from "esbuild";

const script = await readFile("crates/engine/assets/render.js", "utf8");
test("real React hydration and accessible list pagination work without site-specific markup", async ({
  page,
}) => {
  const source = `import React,{useState,useEffect}from'react';import{createRoot}from'react-dom/client';function App(){const[items,setItems]=useState([]),[n,setN]=useState(0);useEffect(()=>{fetch('/data').then(r=>r.json()).then(setItems)},[]);return React.createElement('main',null,React.createElement('div',{role:'list'},items.slice(n*2,n*2+2).map(name=>React.createElement('article',{role:'listitem',key:name},React.createElement('a',{href:'/post/'+name},name)))),React.createElement('nav',{'aria-label':'Pagination'},React.createElement('button',{'aria-label':'Next page',disabled:n>=2,onClick:()=>setN(n+1)},'Next page')))}createRoot(document.getElementById('app')).render(React.createElement(App));`;
  const bundle = buildSync({
    stdin: { contents: source, resolveDir: process.cwd(), loader: "js" },
    bundle: true,
    format: "esm",
    platform: "browser",
    write: false,
    minify: true,
    define: { "process.env.NODE_ENV": '"production"' },
  }).outputFiles[0].contents;
  const server = createServer((req, res) => {
    if (req.url === "/data") {
      res.setHeader("Content-Type", "application/json");
      res.end(
        JSON.stringify(["Orchid", "Fern", "Moss", "Oak", "Pine", "Cedar"]),
      );
      return;
    }
    if (req.url === "/boot.js") {
      res.setHeader("Content-Type", "text/javascript");
      res.end(bundle);
      return;
    }
    res.setHeader("Content-Type", "text/html");
    res.end(
      '<title>React fixture</title><div id="app"></div><script type="module" src="/boot.js"></script>',
    );
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("No fixture port");
  try {
    await page.addInitScript({
      content: `window.__zimsmithRenderOptions={delay_ms:1000,timeout_secs:15,max_html_bytes:8388608};\n${script}`,
    });
    await page.goto(`http://127.0.0.1:${address.port}/`);
    let snapshot: any;
    await expect
      .poll(
        async () => {
          snapshot = await page.evaluate(() =>
            (window as any).__zimsmithCapture.poll(),
          );
          return !!snapshot;
        },
        { timeout: 20000, intervals: [300] },
      )
      .toBe(true);
    expect(snapshot.settled).toBe(true);
    for (const name of ["Orchid", "Fern", "Moss", "Oak", "Pine", "Cedar"])
      expect(snapshot.html).toContain(name);
    expect(snapshot.html).not.toContain("<script");
    expect(snapshot.html).not.toContain('aria-label="Pagination"');
  } finally {
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});
test("button-paginated catalogs retain every product card and remove dead controls", async ({
  page,
}) => {
  const server = createServer((_req, res) => {
    res.setHeader("Content-Type", "text/html");
    res.end(`<title>Catalog</title><div class="block-product-list" id="catalog"><p class="product-list__total-count">6 products</p><select data-qa="product-sort-dropdown"><option>Default</option></select><div class="product-list__content"></div><div class="product-list-pagination"><button data-qa="button-next">Next</button></div></div><script>
  let n=0;const next=document.querySelector('[data-qa=button-next]');function draw(){document.querySelector('.product-list__content').innerHTML=[n*2+1,n*2+2].map(id=>'<a class="product-list__link" href="/product/'+id+'">Product '+id+'<button data-qa="productlistsection-btn-addtobag">Add to bag</button></a>').join('');next.disabled=n===2;}next.onclick=()=>{n++;draw()};draw();</script>`);
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("No fixture port");
  try {
    await page.addInitScript({
      content: `window.__zimsmithRenderOptions={delay_ms:1000,timeout_secs:15,max_html_bytes:8388608};\n${script}`,
    });
    await page.goto(`http://127.0.0.1:${address.port}/`);
    let snapshot: any;
    await expect
      .poll(
        async () => {
          snapshot = await page.evaluate(() =>
            (window as any).__zimsmithCapture.poll(),
          );
          return !!snapshot;
        },
        { timeout: 20000, intervals: [300] },
      )
      .toBe(true);
    expect(snapshot.settled).toBe(true);
    for (let i = 1; i <= 6; i++)
      expect(snapshot.html).toContain("Product " + i);
    expect(snapshot.html).toContain("6 products captured");
    expect(snapshot.html).not.toContain("product-list-pagination");
    expect(snapshot.html).not.toContain("Add to bag");
  } finally {
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});
test("browser capture waits for API content, load-more, and runtime CSS", async ({
  page,
}) => {
  const server = createServer((req, res) => {
    if (req.url === "/data") {
      res.setHeader("Content-Type", "application/json");
      res.end(JSON.stringify(["Orchid", "Fern", "Moss"]));
      return;
    }
    res.setHeader("Content-Type", "text/html");
    res.end(`<html><head><title>Dynamic shop</title><style id="runtime"></style></head><body><div class="block-product-list" id="products"></div><button id="more">Load more</button><script>
    document.querySelector('#runtime').sheet.insertRule('.product { color: rgb(1, 2, 3) }');
    fetch('/data').then(r=>r.json()).then(names=>{ document.querySelector('#products').innerHTML=names.map(name=>'<a class="product" href="/product/'+name+'">'+name+'</a>').join(''); });
    document.querySelector('#more').onclick=function(){ const a=document.createElement('a');a.href='/product/Tree';a.textContent='Tree';document.querySelector('#products').append(a);this.remove(); };
    </script></body></html>`);
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("No fixture port");
  try {
    await page.addInitScript({
      content: `window.__zimsmithRenderOptions={delay_ms:1000,timeout_secs:15,max_html_bytes:8388608};\n${script}`,
    });
    await page.goto(`http://127.0.0.1:${address.port}/`);
    let snapshot: any;
    await expect
      .poll(
        async () => {
          snapshot = await page.evaluate(() =>
            (window as any).__zimsmithCapture.poll(),
          );
          return !!snapshot;
        },
        { timeout: 20000, intervals: [300] },
      )
      .toBe(true);
    expect(snapshot.settled).toBe(true);
    for (const name of ["Orchid", "Fern", "Moss", "Tree"])
      expect(snapshot.html).toContain(name);
    expect(snapshot.html).toContain("rgb(1, 2, 3)");
    expect(snapshot.html).toContain("<base href=");
    expect(snapshot.resource_count).toBeGreaterThan(0);
  } finally {
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});

test("browser capture surfaces API 429s and their Retry-After", async ({
  page,
}) => {
  const server = createServer((req, res) => {
    if (req.url === "/data") {
      res.writeHead(429, { "Retry-After": "7" }).end("Wait");
      return;
    }
    res.setHeader("Content-Type", "text/html");
    res.end(`<title>Store</title><script>fetch('/data')</script>`);
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("No fixture port");
  try {
    await page.addInitScript({
      content: `window.__zimsmithRenderOptions={delay_ms:1000,timeout_secs:10,max_html_bytes:8388608};\n${script}`,
    });
    await page.goto(`http://127.0.0.1:${address.port}/`);
    let snapshot: any;
    await expect
      .poll(
        async () => {
          snapshot = await page.evaluate(() =>
            (window as any).__zimsmithCapture.poll(),
          );
          return !!snapshot;
        },
        { timeout: 10000 },
      )
      .toBe(true);
    expect(snapshot.rate_limit.status).toBe(429);
    expect(snapshot.rate_limit.retry_after).toBe("7");
  } finally {
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});
