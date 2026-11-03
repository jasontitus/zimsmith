# Archive format and crawl policy

[Back to Zimsmith](../README.md)

## Search and archive format

[zimru](https://github.com/jasontitus/zimru) is linked directly. Content is passed in chunks, Zstd compression uses a small in-flight byte budget, precompressed media stays raw, and the file is finalized with tables and a checksum before being renamed to `archive.zim`.

Every archive has an HTML contents page and Unicode-tokenized inverted search index over captured text and media descriptions. Audio/video is not transcribed. Portable search runs on the archive home page, rather than in Kiwix's global search bar.

For Kiwix-native title/full-text search, enable [xapianbuilder](https://github.com/jasontitus/xapianbuilder):

```sh
# macOS: brew install xapian icu4c pkgconf
# Ubuntu: install libxapian-dev libicu-dev pkg-config
npm run tauri dev -- --features kiwix-index
npm run tauri build -- --features kiwix-index
```

Databases are embedded uncompressed as `X/fulltext/xapian` and `X/title/xapian`. Mobile can use this feature with target Xapian/ICU libraries and pkg-config metadata; cross-building those dependencies is not automated here. Portable search remains available without them.

## Crawl policy and recovery

- Website discovery stays within the starting origin and URL path, with explicit item/depth limits and a bounded queue.
- robots.txt is checked per origin, including asset hosts. Missing policies permit crawling; denial/unreadable policies fail closed. Declared crawl delay is conservatively honored.
- Website pages and assets from ordinary hosts share the configured per-origin spacing across jobs. Linked assets from recognized static CDNs (including Google Fonts, jsDelivr, cdnjs, and YouTube thumbnail hosts) are fetched sequentially without the artificial page delay. The exact-host list is shared by native and browser engines in `assets/static-cdns.json`; a CDN-looking hostname, a font filename, or a general cloud-storage domain does not qualify by itself. Explicit robots crawl delays still apply, including before the first asset. Queued requests recheck extended host cooldowns before receiving a slot; scheduling or bookkeeping delays cannot consume a declared interval between sequential requests. 429/5xx responses retain bounded retries, exponential backoff/jitter, numeric or HTTP-date `Retry-After`, and saved cooldowns, even for fast CDN assets.
- Extractors use their provider-specific retry behavior. RustyPipe queries are paced; yt-dlp receives sleep/retry/fragment limits. Extractor internals are not governed by the website robots engine. Direct transfers honor rate-limit headers.
- Media streams to partial files. Native range continuation requires a strong ETag and matching range; otherwise it restarts. yt-dlp retains its own partial downloads.
- Manifests are replaced atomically. Filesystem, HTTP, extraction, and packaging failures appear in the job/log. Retry reuses completed items; failed assets prevent silently publishing incomplete archives.

Jobs are stored in the OS app-data directory and include source URLs, cached HTML/assets/media, metadata, checkpoints, and archives.
