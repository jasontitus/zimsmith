// Local fixture for platform WebView instrumentation; contains no third-party site data.
import { createServer } from "node:http";
import { buildSync } from "esbuild";
const source = `import React,{useState,useEffect} from 'react';import{createRoot}from'react-dom/client';
function App(){const[items,setItems]=useState([]),[page,setPage]=useState(0);useEffect(()=>{fetch('/data').then(r=>r.json()).then(setItems)},[]);
const paginated=location.pathname==='/pages',shown=paginated?items.slice(page*2,page*2+2):items;
return React.createElement('main',null,React.createElement('h1',null,'Framework-rendered collection'),React.createElement('div',{role:'list','aria-setsize':items.length},shown.map(item=>React.createElement('article',{role:'listitem',key:item},React.createElement('a',{href:'/post/'+item},item),React.createElement('p',null,'Content arrived through an API after React hydration.')))),paginated&&React.createElement('nav',{'aria-label':'Pagination'},React.createElement('button',{'aria-label':'Next page',disabled:page>=2,onClick:()=>setPage(page+1)},'Next page')))}
createRoot(document.getElementById('app')).render(React.createElement(App));`;
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
  if (req.url === "/limited") {
    res.writeHead(429, { "Retry-After": "7" }).end("Wait");
    return;
  }
  if (req.url === "/data") {
    res.setHeader("Content-Type", "application/json");
    res.end(JSON.stringify(["Orchid", "Fern", "Moss", "Oak", "Pine", "Cedar"]));
    return;
  }
  if (req.url === "/boot.js") {
    res.setHeader("Content-Type", "text/javascript");
    res.end(bundle);
    return;
  }
  if (req.url === "/robots.txt") {
    res.end("User-agent: *\nDisallow:\n");
    return;
  }
  res.setHeader("Content-Type", "text/html");
  res.end(
    '<!doctype html><html><head><title>Hydrated collection</title><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><div id="app">Loading collection</div><script type="module" src="/boot.js"></script></body></html>',
  );
});
const port = Number(process.env.ZIMSMITH_RENDER_TEST_PORT || 9786);
server.listen(port, "127.0.0.1", () =>
  process.stdout.write(`Local rendering fixture: ${port}\n`),
);
