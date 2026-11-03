# Using Zimsmith

[Back to Zimsmith](../README.md)

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

YouTube uses the [TeamPiped RustyPipe fork](https://github.com/TeamPiped/rustypipe) in-process, vendored at revision `3f1491263929c22036429b665e370fdd2f066017`. [Source provenance and compatibility changes](../vendor/rustypipe/ZIM-STUDIO.md) record the HTTP dependency migration and JSON dependency pin needed to build that snapshot. The fork's JavaScript solver runs through embedded QuickJS; mobile builds generate matching QuickJS bindings. Combined streams and separate MP4 video/audio downloads are supported. During packaging, separate tracks are combined locally with a pinned Rust MP4 reader/writer into one progressive MP4. Encoded samples, presentation timing, frame ordering, and audio priming edits are preserved without transcoding or FFmpeg. The exported page uses one native video player and one playback clock, including in kiwix-apple. Existing downloads receive the fix through **Rebuild ZIM**, then **Open ZIM** or **Save ZIM file**; old exported files keep their embedded player. MP4 parsing and sample buffers have explicit resource limits, and the job can pause during combining. Provider challenges, removed content, expired URLs, and extractor changes can still cause failures. Desktop can fall back to local yt-dlp; mobile reports a retryable error. Generic mobile provider parity with yt-dlp remains unfinished.

YouTube audio selection prefers the provider's **original** track before bitrate or codec quality; it does not pick a dub based on the archive language. Multi-language streams without a verifiable original fail with an item error instead of choosing an arbitrary language. The Activity log records the chosen language/track. Language-specific cache identities prevent audio tracks with the same YouTube format number from being reused across languages. Combined MP4 inputs are remuxed with the selected audio so an embedded default dub cannot override it.

For an existing YouTube collection with the wrong language, open its **Details → Repair original audio** in the native app. This keeps the approved selection, re-resolves audio without rescanning, reuses matching saved video files, and rebuilds the ZIM. Audio or a changed video format may need downloading again. Exported ZIM copies stay unchanged; save/open the repaired revision after it completes. The action is available for completed, failed, or paused jobs after scanning, and preserves server cooldowns and the previous archive until a replacement succeeds. **Rebuild ZIM** alone changes the template using cached files and cannot repair an already downloaded dub.

The crawler captures server-rendered HTML. It does not execute a headless browser, capture authenticated/JavaScript-only applications, DRM, or live streams. Source scripts and event handlers are removed from website pages; an archive CSP blocks uncaptured subresource requests. External navigation links remain online links.
