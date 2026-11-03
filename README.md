# Zimsmith

A Tauri 2 app for turning websites and media collections into searchable offline ZIM archives. Crawling, checkpoints, content preparation, search, and ZIM writing run **on the device**, including iOS and Android. Installed apps do not need a server or account.

## Desktop

Install Node.js 20+, Rust, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
npm ci
npm run tools:update  # latest yt-dlp source, optional
npm run tauri dev
# installer builds:
npm run tauri build
```

Development desktop builds automatically prefer `.tools/yt-dlp` and run it with Python 3.11+, falling back to `yt-dlp` on PATH. Release builds use the explicit source override or PATH rather than embedding a developer checkout path. `ZIMSTUDIO_YTDLP` overrides the executable; `ZIMSTUDIO_YTDLP_SOURCE` selects another source directory; `ZIMSTUDIO_PYTHON` selects Python. Submitted URLs are passed as arguments, never interpolated into shell commands. Release installers do not bundle Python/yt-dlp yet; these are optional runtime dependencies for additional desktop providers.

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

## Workflow

1. Enter a hostname (such as `cnn.com`), website URL, YouTube channel/playlist/search/video URL, or direct media file. Hostnames default to HTTPS; explicit HTTP URLs and ports are preserved. Set item/depth limits, request spacing, language, and maximum individual file size.
2. Scan. Websites cache HTML for discovery; media sources fetch metadata. With the default preferences, complete, error-free scans of **up to 100 items** proceed directly to downloading and building.
3. Review jobs above the automatic threshold, incomplete scans, and scans with failed items. Select entries and confirm **Download & build ZIM**. Only approved entries enter the final contents and search index.
4. Pause or resume from Activity. Completed items are checkpointed; interrupted jobs reopen paused. Failures retain a reason and retry action. Use **Edit crawl** on a failed or paused job to change its source, crawl limits, download settings, or archive details, then **Save & retry**. Cancel leaves the job unchanged. Source/type, item/depth limit, or asset-policy changes restart discovery; other changes resume the saved phase. Downloaded payloads are retained, with the previous manifest kept for recovery. Editing retains server backoff.
5. Save the ZIM and open it in Kiwix. The archive home page has contents and offline full-text search.

The initial page may automatically adopt a canonical `www` hostname or HTTPS upgrade on the same site/path. Other cross-site, path-scope, port, and HTTPS-downgrade redirects still require an explicit source URL.

YouTube and direct media URLs, including ones pasted without a scheme, automatically select media scanning. The engine also corrects these URLs when submitted by older clients. For a previously failed HTML scan of a YouTube URL, use **Retry & resume** to restart it with the media extractor.

In **Settings → Download confirmation**, turn automatic downloads on or off and set the item threshold (1–5,000, inclusive). Preferences are saved on the device and copied into new jobs, so changing them does not alter existing scans or recovered jobs. Older saved jobs continue to require manual approval. The threshold counts pages or media entries; linked assets are additional. It is **not a total download-size or memory limit**, and media sizes can be unknown. Large downloads remain possible with few items. Scan limits still apply; truncated collections require review even if their discovered count is small.

## Archive appearance

Choose an index-page template in **Settings → Archive appearance** and select **Save archive style**:

- **Atlas**: a bright, spacious card grid with violet accents.
- **Field Notes**: warm paper, serif headings, and a numbered editorial list.
- **Midnight**: a dark library with mint accents.
- **Screening Room**: a video gallery inspired by [TED’s ZIM gallery](https://github.com/openzim/ted/tree/main/src/ted2zim/templates), with offline thumbnails, duration badges, search, and a spacious cinema player. Tap a thumbnail to play immediately in the gallery overlay; tap the title or description to open the detail page. The thumbnail is a Play button rather than a page link, so Kiwix does not offer a link preview when it is closed. **Back to gallery** stops playback and returns focus to the thumbnail. **Next video** plays the next visible video, following any search filter; it is disabled on the last video. Finishing a clip stays in the player without opening a page or automatically advancing. Artwork is saved after approval, capped at 2 MB per image, and uses the asset pacing policy, robots rules, and 429 backoff. Missing artwork uses a placeholder and does not fail the video download.

The static style preview is rendered by the same Rust template code as the exported ZIM. The selection is saved on the device and copied into each new job; subsequent setting changes do not change that job's eventual export. For a downloaded collection, open **My archives → Details**, choose an **Archive template**, and tap **Rebuild ZIM**. This reuses saved pages, video, audio, and search text, and supports pause/resume during packaging; Screening Room may fetch missing artwork (including for older YouTube jobs). A rebuild preserves the previous ZIM until the replacement is complete. Open/Save uses a new revision filename so existing saved copies stay unchanged. Existing settings/manifests default to Atlas, and previously generated files are not modified.

Templates apply to the contents/search page, generated audio/video pages, and the collection navigation bar on captured website pages. Original website layouts are preserved. All styles, search scripts, and bundled DM Sans/Manrope fonts (including their license texts) are written inside the archive. Serif headings use system fonts. There are no remote font, image, or CDN dependencies. Layouts adapt to phones, support keyboard navigation and reduced motion, and include styled search results and an empty-results state. Search still requires a reader that permits JavaScript; contents links remain available without it.

## Platform support

| Capability                                           | macOS / Windows / Linux | iOS / Android                       |
| ---------------------------------------------------- | ----------------------- | ----------------------------------- |
| Local HTML crawl, preview, assets, link rewriting    | Included                | Included                            |
| Native zimru ZIM writer                              | Included                | Included                            |
| Portable inverted search index and contents          | Included                | Included                            |
| Native YouTube channels, playlists, searches, videos | Included                | Included                            |
| Direct MP4, WebM, MP3, M4A, Ogg, WAV, MOV links      | Included                | Included                            |
| Additional yt-dlp providers                          | Optional local adapter  | Not yet included                    |
| Native Xapian title/full-text index                  | Optional build feature  | Requires cross-built Xapian/ICU     |
| File export                                          | Downloads folder        | iOS Files / Android document picker |

Mobile jobs run in the foreground. Keep the app open while crawling. The app requests a pause when hidden and recovers its persisted manifest after OS termination. It does not claim indefinite background execution. Interrupted packaging restarts from cached inputs; unfinished ZIMs are not published as completed archives.

YouTube uses the [TeamPiped RustyPipe fork](https://github.com/TeamPiped/rustypipe) in-process, vendored at revision `3f1491263929c22036429b665e370fdd2f066017`. [Source provenance and compatibility changes](vendor/rustypipe/ZIM-STUDIO.md) record the HTTP dependency migration and JSON dependency pin needed to build that snapshot. The fork's JavaScript solver runs through embedded QuickJS; mobile builds generate matching QuickJS bindings. Combined streams and separate MP4 video/audio downloads are supported. During packaging, separate tracks are combined locally with a pinned Rust MP4 reader/writer into one progressive MP4. Encoded samples, presentation timing, frame ordering, and audio priming edits are preserved without transcoding or FFmpeg. The exported page uses one native video player and one playback clock, including in kiwix-apple. Existing downloads receive the fix through **Rebuild ZIM**, then **Open ZIM** or **Save ZIM file**; old exported files keep their embedded player. MP4 parsing and sample buffers have explicit resource limits, and the job can pause during combining. Provider challenges, removed content, expired URLs, and extractor changes can still cause failures. Desktop can fall back to local yt-dlp; mobile reports a retryable error. Generic mobile provider parity with yt-dlp remains unfinished.

The crawler captures server-rendered HTML. It does not execute a headless browser, capture authenticated/JavaScript-only applications, DRM, or live streams. Source scripts and event handlers are removed from website pages; an archive CSP blocks uncaptured subresource requests. External navigation links remain online links.

## Mobile builds

Install Xcode for iOS, or Android SDK, NDK 28+, and JDK 17/21 for Android. Install targets in the same rustup toolchain used for compilation:

The fork's TLS dependency also builds native code: install CMake, Perl, and Clang/libclang (plus NASM on Windows). The mobile wrapper supplies the Android NDK variables needed by that build.

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim aarch64-linux-android
npm run ios -- init --ci
npm run android -- init --ci
npm run ios -- dev
npm run android -- dev
# Android arm64 debug APK:
npm run android -- build --debug --apk --ci
```

Windows source builds should use the current `main` branch. The initial `v0.1.2` source tag predates the MSVC MD5 backend compatibility fix; the Android APK attached to that preview is unaffected. The original RustCrypto MD5 algorithm and upstream test vectors are preserved in the compatibility patch.

The mobile wrapper selects rustup's compiler explicitly, statically builds LZMA, and configures QuickJS binding generation with the Android NDK sysroot. Set `ANDROID_HOME`, `NDK_HOME`, and `JAVA_HOME` for nonstandard installations. Android builds currently select arm64; other ABIs need matching bindgen/NDK configuration.

Signed iOS builds require your `APPLE_DEVELOPMENT_TEAM`. Generated projects under `src-tauri/gen/` can be regenerated. **Save ZIM file** opens the native Files/document picker on iOS and Android; choose Downloads, iCloud Drive, or another folder. Canceling keeps the completed archive available for another attempt. Desktop exports go to Downloads with the archive title in the filename.

Tap a completed collection or **Open ZIM** to hand it to a viewer. Desktop and Android use the system file association; iOS presents the installed apps that support ZIM files. A separate **Details** action keeps job information accessible. Saving and opening do not remove the internal recovery copy. Large archives are staged with a hard link when supported, so opening the picker does not first require another full archive copy.

The app viewport is fixed to its native scale. iOS and Android webviews disable pinch zoom, and mobile inputs are at least 16px to prevent focus zoom. Exported website pages and archive templates keep their normal reader behavior.

Active jobs show their current operation, stage progress, current-file bytes, request/failure counts, and time since the last activity. Discovery and finalization use indeterminate progress when a total is unknown. Transfer updates stay in memory between normal checkpoints. Activity logs use larger, higher-contrast text, with separate warning and error colors.

YouTube media transfers use bounded ranges, including the provider's range-query protocol and signed file size. Requests do not extend past the resource end. A denied request stops with an error rather than repeatedly requesting the same forbidden URL. The app's preferred player profile is the current visionOS profile recorded in the RustyPipe provenance file.

## Direct Android releases and updates

The direct-download Android flavor includes update discovery and a browser handoff. Regular `npm run android -- build ...` builds leave it disabled for store distribution; enabling a settings toggle cannot turn it on in those binaries.

```sh
# Use your own HTTPS domain or Firebase Hosting domain. Omit the environment
# variable during private testing and set the feed in Settings → App updates.
ZIMSMITH_UPDATE_FEED_URL=https://YOUR_HOST/updates/android.json npm run android:direct -- build --apk --ci
```

The installed app reads its actual Android package/version code, supported ABIs, SDK level, and signing-certificate fingerprint. A compatible feed must identify the same app, direct channel, and signing key, and offer a strictly newer version code. Automatic checks occur on opening/foregrounding and hourly while open, with a persisted daily limit. Manual checks bypass that daily interval but still honor server cooldowns. Responses are capped at 64 KB with a 15-second request timeout; redirects must stay HTTPS on the same origin. Errors keep the previous valid offer and appear in update settings. **Later** dismisses that version until a manual check or a newer release.

**Download update** opens an HTTPS URL in the external browser. It is disabled while scanning, downloading, packaging, or pausing; native code also checks before handoff. Android handles the downloaded APK and asks the user to approve installation. Zimsmith requests no app-install permission and never downloads or silently installs executable updates itself. The normal browser's “install unknown apps” approval may be needed. Keep the same package ID and production signing key across releases, and increment the Android version code. The first public preview is 0.1.2 / version code 1002; older APKs without this updater need one manual update.

**Settings → App updates** has automatic/manual checks and an advanced HTTPS feed address. An empty override uses the build-time default. Changing addresses clears cached release metadata. Checks request release information only; crawl URLs, content, and job data are not sent. The host still receives ordinary HTTP request information such as the device's IP address. The app checks the feed's declared signing identity; Android verifies the actual downloaded APK signature before installing.

### GitHub Pages and Releases

The public distribution uses [GitHub Pages](https://jasontitus.github.io/zimsmith/) for the download page and update feed, and [GitHub Releases](https://github.com/jasontitus/zimsmith/releases) for immutable, checksum-named APKs. Users do not need a GitHub account to download. The static site in `site/` is deployed by `.github/workflows/pages.yml`; APKs are release assets and are not stored in Git history.

```sh
ZIMSMITH_UPDATE_FEED_URL=https://jasontitus.github.io/zimsmith/updates/android.json npm run android:direct -- build --apk --ci
node scripts/sign-android-release.mjs src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk output/builds/Zimsmith-<version>-android-arm64.apk
npm run release:android -- --apk output/builds/Zimsmith-<version>-android-arm64.apk --base-url https://jasontitus.github.io/zimsmith/ --download-base-url https://github.com/jasontitus/zimsmith/releases/download/v<version>/ --source-url https://github.com/jasontitus/zimsmith/tree/v<version> --notes-file releases/android-<version>.txt
```

Copy the generated HTML, icon, checksum, and feed from `output/hosting/` into `site/`, leaving APKs out of the site. Upload only the exact APK filename listed in `output/hosting/updates/android.json` to the matching release tag before pushing the updated site. Review release notes, run the publication audit, and verify the live feed and download after deployment. Pages uses its normal cache behavior; there is no custom server requirement or Firebase dependency.

The signing helper reads `android-release.p12` and `android-release.password` from the private user configuration directory `.config/zimsmith/signing`, or from `ZIMSMITH_SIGNING_DIR`. These files must never be committed or uploaded as release assets. Retain an offline backup of the signing key and password: future Android updates require that same key. The certificate uses the generic project identity. Public preview APKs cannot update earlier development-signed APKs in place; export archives before replacing a development installation.

Mobile release builds remap local home-directory paths in Rust and native C/C++ source locations. Before the first public upload, source, icons, and release artifacts were reviewed for private information; see [the publication review](docs/PUBLICATION_REVIEW.md). Keep private audit reports outside the repository.

### Prepare a portable static download site

Sign the release APK with your production key first, using the Android SDK's `zipalign` and `apksigner`, or configure release signing in your build environment. Signing credentials are not stored here. Then:

```sh
npm run release:android -- --apk /absolute/path/to/signed.apk --base-url https://YOUR_HOST --notes-file /absolute/path/to/release-notes.txt
# Inspect output/hosting locally, then deploy to the project you choose:
firebase deploy --only hosting --project YOUR_PROJECT_ID
```

The script verifies the APK signature and inspects the actual package ID, version, minimum SDK, ABI, and certificate using Android SDK build tools. It refuses debug/development signing unless `--allow-development-key` is explicitly supplied for private testing. Output includes a branded download page, immutable checksum-named APK, SHA-256 checksum, and `updates/android.json` release feed. Re-running replaces that ABI's feed entry while retaining other ABI entries and old APKs; use the same output directory when maintaining multiple ABIs. Release notes are plain text and HTML-escaped. The generated page can also be tracked as an HTML source in [Obtainium](https://wiki.obtainium.imranr.dev/app_tracking/).

`firebase.json` serves that directory with uncached feeds and immutable APK downloads. No Firebase project, hosting domain, production signing key, or deployment is selected automatically. The output is ordinary static files and can move to any HTTPS host. Using a domain you control lets you move hosting through DNS; the advanced feed setting also allows an address change without first replacing the app.

Firebase Hosting is not a guarantee against suspension: its [applicable terms](https://firebase.google.com/terms/) include the [Google APIs Terms](https://developers.google.com/terms/), which cover third-party rights, other Google terms, and termination. Keep hosting portable rather than assuming Firebase has the same approval rules as Google Play or that it cannot remove a download site.

Validation:

```sh
cargo test -p zim-studio --lib
npx playwright test tests/android-updates.spec.ts tests/native-files.spec.ts
ZIMSMITH_TEST_APK=/absolute/path/to/development-signed.apk node --test scripts/prepare-android-release.test.mjs
```

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
- Website pages and assets from ordinary hosts share the configured per-origin spacing across jobs. Linked assets from recognized static CDNs (including Google Fonts, jsDelivr, cdnjs, and YouTube thumbnail hosts) are fetched sequentially without the artificial page delay. The exact-host list is shared by native and browser engines in `assets/static-cdns.json`; a CDN-looking hostname, a font filename, or a general cloud-storage domain does not qualify by itself. Explicit robots crawl delays still apply, including before the first asset. 429/5xx responses retain bounded retries, exponential backoff/jitter, numeric or HTTP-date `Retry-After`, and saved cooldowns, even for fast CDN assets.
- Extractors use their provider-specific retry behavior. RustyPipe queries are paced; yt-dlp receives sleep/retry/fragment limits. Extractor internals are not governed by the website robots engine. Direct transfers honor rate-limit headers.
- Media streams to partial files. Native range continuation requires a strong ETag and matching range; otherwise it restarts. yt-dlp retains its own partial downloads.
- Manifests are replaced atomically. Filesystem, HTTP, extraction, and packaging failures appear in the job/log. Retry reuses completed items; failed assets prevent silently publishing incomplete archives.

Jobs are stored in the OS app-data directory and include source URLs, cached HTML/assets/media, metadata, checkpoints, and archives.

## Browser development and checks

A local HTTP bridge supports browser UI development only. Native apps, including phones, always use local Tauri IPC:

```sh
# terminal 1:
npm run dev:api
# terminal 2:
npm run dev
# terminal 3:
npx playwright install chromium
npm run test:e2e
```

The development API binds `127.0.0.1:8788`, uses `.zim-studio/` for storage, and permits Vite on port 1420. `VITE_API_URL` overrides the browser address. Non-loopback binding requires a 24+ character `ZIMSTUDIO_TOKEN`; extra browser origins require `ZIMSTUDIO_ORIGINS`.

```sh
npm test
cargo test -p zim-engine --features kiwix-index
npm run build
npm run ios -- check --offline
npm run android -- check --offline
npm run format:rust:check
```

Tests cover scope, HTML/CSS rewriting, script removal, robots exclusions, no premature asset download, pause/resume, 429/restart recovery, media URL detection and recovery from an old HTML scan, current/classic YouTube playlist layouts, bounded range assembly, fail-fast 403 handling, and reading completed ZIMs. Browser tests verify changing progress/byte counters, cooldown and error states, and readable log contrast as well as the archive workflows. Xapian tests assert embedded database entries. Opt-in native YouTube checks are available with `cargo test -p zim-engine --test workflow youtube_ -- --ignored --nocapture --test-threads=1`: playlist/video/channel/search metadata, plus retrieval of a small prefix of the selected video and audio streams. `cargo test -p zim-engine downloads_youtube_video_and_builds_zim -- --ignored --nocapture` downloads one complete video and audio track and validates a built ZIM. Provider access varies with the network and current YouTube behavior. Full playback still needs device testing.

Template tests also check saved selection and job-level style snapshots, escaped metadata, bundled fonts inside real ZIMs, all four desktop/mobile layouts, working search and clear/empty states, archive navigation, and zero external subresource requests. Visual test outputs are under `test-results/archive-templates/`.

## Sources updated on 2026-11-02

- `yt-dlp`: cloned and pulled with `git pull --ff-only`; version `2026.10.26`, revision `b77c9304e1c4220cca450c583491904daf8455af`.
- Existing `zimwriterfs`: pulled and up to date at `15200b1017654d9d5fd75dd31491c7a25761fb18`. Upstream is archived; the app uses native zimru instead.
- Existing `zimru` / `xapianbuilder`: pulled and up to date. Cargo pins `061afbc2920c78fc4ecb7211cb9ef4f3655dba35` / `13ff368991850f35ecb2e60037c6385d9992cbdc`, so builds do not require sibling directories.
- `RustyPipe`: TeamPiped fork snapshot `3f1491263929c22036429b665e370fdd2f066017`, with provenance and dependency compatibility changes under `vendor/rustypipe/`.

## Code layout

- `src/`: React interface and native IPC adapter.
- `src-tauri/`: desktop/mobile app and export commands.
- `crates/engine/`: crawler, extraction, persistence, rewriting, search, and ZIM writer.
- `crates/mobile-files/`: native mobile save pickers, ZIM viewer handoff, and webview zoom configuration.
- `vendor/rustypipe/`: pinned TeamPiped RustyPipe library snapshot.
- `vendor/zimru/`: pinned MIT-licensed ZIM library source with Unix-only memory-map hints gated for Windows.
- `vendor/md-5/`: RustCrypto 0.10.6 with an MSVC backend-selection fix; upstream algorithm code and licenses retained.
- `crates/worker/`: optional browser-development API.
- `tests/`: browser workflow tests.

Dependencies include GPL-3.0 RustyPipe and optional GPL-3.0-or-later xapianbuilder; their repositories contain the license texts.

## License

Zimsmith's original source is licensed under **GPL-3.0-only**; see [LICENSE](LICENSE). Vendored libraries, fonts, and other third-party material retain their own licenses and attribution. The RustyPipe source and its license are preserved under `vendor/rustypipe/`. Browser and extension packages include the project license and third-party notices. The public release source is available under the corresponding GitHub release tag.
