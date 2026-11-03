# Browser and extension builds

[Back to Zimsmith](../README.md)

## Browser and extension builds

The browser target runs entirely on the user's device: a dedicated Web Worker crawls, IndexedDB saves atomic job checkpoints, and the origin-private filesystem stores downloads and finished archives. It uses no build server, native helper, Python, or localhost API. The existing HTTP bridge remains only for native-UI development.

For arbitrary websites, use the **Chrome/Edge Manifest V3 extension**. Its full-page workspace can fetch sources after you grant website access; an ordinary website remains subject to the source's CORS policy.

```sh
npm run extension:build
# Load output/extension with "Load unpacked" in chrome://extensions
# or edge://extensions (Developer mode). Click the Zimsmith toolbar icon.
# Shareable ZIP: output/builds/Zimsmith-<version>-chromium-extension.zip

npm run web:dev   # browser-only development on http://127.0.0.1:1422
npm run web:build # static files in output/web; serve over HTTPS or localhost
```

The extension requests optional HTTP/HTTPS host access when starting a crawl. Downloads omit login cookies, no content script is injected, and there is no message bridge from websites into its privileged fetch code. Only one workspace tab may own the browser engine at a time. Keep that tab open while jobs run; reopening recovers interrupted jobs as **Paused**. Pause/resume, persistent 429/Retry-After cooldowns, robots rules/crawl delay, review and automatic-download settings, failed-crawl editing, templates, and rebuilding from cached files are supported. Strong ETags allow partial media downloads to resume; changed files or servers without a strong validator restart safely. **Save ZIM file** streams the stored file through a browser download, using the browser's Downloads folder or save dialog.

The browser build currently supports server-rendered HTML websites and **direct MP4/WebM/MOV/MP3/M4A/Ogg/WAV files**. YouTube/other collection extraction has **not** been ported from native RustyPipe/yt-dlp; the UI reports this explicitly rather than attempting an empty HTML crawl. This first extension package targets desktop Chromium. Firefox needs a separate manifest/test pass, and an iOS Safari extension needs an Apple app wrapper; the existing native phone apps remain independent.

Browser requests stop at redirects, since redirect destinations can be opaque and must not be fetched before scope/robots checks. Open a redirecting source in a normal tab and use its final URL in **Edit crawl**. Redirecting assets also stop the job rather than publishing an incomplete archive. Headers hidden by a normal website's CORS policy can prevent safe byte-range continuation; the browser then restarts that file. Extension requests can inspect these headers directly.

A streaming JavaScript writer emits standard **ZIM 6.3 with uncompressed clusters**, modern title listings, metadata, illustrations, 64-bit file positions, and an incremental MD5 checksum. This avoids porting zimru's native filesystem/threading/compression stack to WASM. Archives can be larger than native Zstd builds. Browser-generated fixtures are independently read and structurally/checksum-validated with zimru. The four index styles reuse generated native template shells and the same bundled CSS, fonts, search, and gallery player. Refresh shells with `npm run web:templates` after changing the Rust renderer.

Media and archive payloads stream to browser storage with bounded buffers; the whole ZIM is never assembled in memory. Parsing is limited to 8 MB per HTML file and 2 MB per stylesheet. Search uses the first 50,000 extracted text characters per page, with a one-million-posting budget; larger indexes fail with a smaller-collection suggestion. Metadata and index tables still use memory. Browser storage quotas and disk space apply even with an extension's `unlimitedStorage` permission. Clearing site/extension data or uninstalling removes internal recovery files, so export finished ZIMs. Browser builds include the portable in-archive search rather than native Xapian indexes.

```sh
npm run test:browser
```
