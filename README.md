# Zimsmith

Turn websites and media collections into searchable offline [ZIM archives](https://openzim.org/), ready to open in Kiwix. Crawling and archive creation run on your device. No server or account required.

**[Download Android APK](https://jasontitus.github.io/zimsmith/)** · **[Releases](https://github.com/jasontitus/zimsmith/releases)**

## What it does

- Crawls websites, YouTube playlists/channels/searches/videos, and direct media links.
- Scans first, then lets you review what to download. Collections up to 100 items can start automatically; the threshold is configurable.
- Pauses, resumes, checkpoints progress, and respects rate limits and server cooldowns.
- Builds a table of contents, offline search, and a ZIM with a choice of four templates, including a video gallery.
- Exports to Downloads or the phone’s file picker, and opens archives in a compatible viewer.

## Use it

Enter a URL or hostname, scan, review when prompted, then **Download & build ZIM**. Keep the mobile app open while jobs run.

You can also [share a link from your browser](docs/SHARING.md) to Zimsmith on Android and iOS.

Change an existing archive’s style with **Details → Rebuild ZIM**. For older YouTube downloads with the wrong language, use **Details → Repair original audio**, then save/open the rebuilt archive.

## Platforms

| Platform                            | Support                                                         |
| ----------------------------------- | --------------------------------------------------------------- |
| macOS, Windows, Linux, iOS, Android | Native website crawling, YouTube extraction, and ZIM creation   |
| Desktop Chromium extension          | Website crawling after granting site access; direct media links |
| Browser                             | Sites that allow CORS; direct media links                       |

YouTube uses the [TeamPiped RustyPipe fork](https://github.com/TeamPiped/rustypipe). Desktop can optionally use yt-dlp for additional sites. Browser/extension builds do not yet extract YouTube collections. Website capture supports server-rendered HTML; authenticated apps, DRM, and live streams are unsupported.

## Build

Install Node.js 20+, Rust, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
npm ci
npm run tauri dev
npm run tauri build
```

For mobile setup, optional yt-dlp, and testing, see the [development guide](docs/DEVELOPMENT.md).

## Documentation

- [Usage, templates, and audio repair](docs/USAGE.md)
- [Browser and extension builds](docs/BROWSER.md)
- [Android releases and updates](docs/ANDROID_RELEASES.md)
- [Archive format, search, and crawl policy](docs/ARCHITECTURE.md)
- [Publication and privacy review](docs/PUBLICATION_REVIEW.md)

## License

[GPL-3.0-only](LICENSE). Third-party libraries and assets retain their own licenses and attribution.
