# Building and developing Zimsmith

[Back to Zimsmith](../README.md)

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

## Mobile builds

Install Xcode for iOS, or Android SDK, NDK 28+, and JDK 17/21 for Android. Install targets in the same rustup toolchain used for compilation:

The iOS Share Extension also requires XcodeGen (`brew install xcodegen`) and matching App Group provisioning for the app and extension; see [sharing](SHARING.md).

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

Windows source builds should use `v0.1.3` or the current `main` branch. The initial `v0.1.2` source tag predates the MSVC MD5 backend compatibility fix; the Android APK attached to that preview is unaffected. The original RustCrypto MD5 algorithm and upstream test vectors are preserved in the compatibility patch.

The mobile wrapper selects rustup's compiler explicitly, statically builds LZMA, and configures QuickJS binding generation with the Android NDK sysroot. Set `ANDROID_HOME`, `NDK_HOME`, and `JAVA_HOME` for nonstandard installations. Android builds currently select arm64; other ABIs need matching bindgen/NDK configuration.

Signed iOS builds require your `APPLE_DEVELOPMENT_TEAM`. Generated projects under `src-tauri/gen/` can be regenerated. **Save ZIM file** opens the native Files/document picker on iOS and Android; choose Downloads, iCloud Drive, or another folder. Canceling keeps the completed archive available for another attempt. Desktop exports go to Downloads with the archive title in the filename.

Tap a completed collection or **Open ZIM** to hand it to a viewer. Desktop and Android use the system file association. On iOS, choose a ZIM reader or another destination from the system share sheet, which opens at the bottom on iPhone and as a centered popover on iPad. A separate **Details** action keeps job information accessible. Saving and opening do not remove the internal recovery copy. Large archives are staged with a hard link when supported, so opening the picker does not first require another full archive copy.

The app viewport is fixed to its native scale. iOS and Android webviews disable pinch zoom, and mobile inputs are at least 16px to prevent focus zoom. Exported website pages and archive templates keep their normal reader behavior.

Active jobs show their current operation, stage progress, current-file bytes, request/failure counts, and time since the last activity. Discovery and finalization use indeterminate progress when a total is unknown. Transfer updates stay in memory between normal checkpoints. Activity logs use larger, higher-contrast text, with separate warning and error colors.

YouTube media transfers use bounded ranges, including the provider's range-query protocol and signed file size. Requests do not extend past the resource end. A denied request stops with an error rather than repeatedly requesting the same forbidden URL. The app's preferred player profile is the current visionOS profile recorded in the RustyPipe provenance file.

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

The opt-in [live-site matrix](CRAWL-COMPATIBILITY.md) samples news, blogs, science, and documentation with bounded crawl budgets. WARC regression fixtures are generated locally; no captured third-party content is checked into the repository. Run `cargo test -p zim-engine --test warc_import --locked` and `npm run test:browser` for import coverage. WARC parsing uses the [Rust warc crate](https://docs.rs/warc/0.4.0/warc/) and [Webrecorder warcio.js](https://github.com/webrecorder/warcio.js), with the distributed license texts and package license declarations retained in `public/WARC-LICENSES.txt`.

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
