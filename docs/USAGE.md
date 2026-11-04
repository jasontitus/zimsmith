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

The crawler captures server-rendered HTML (up to 8 MB per page). It does not execute a headless browser, capture authenticated/JavaScript-only applications, DRM, or live streams. Source scripts and event handlers are removed from website pages; an archive CSP blocks uncaptured subresource requests. External navigation links remain online links.

### Recover or remove a failed website crawl

Progress shows the actual request, including robots.txt checks. Each robots.txt request times out after 15 seconds; an unavailable policy is checkpointed for 60 seconds so Resume and other assets do not immediately repeat it. Successful policies are cached for an hour. Server cooldowns and robots exclusions still apply.

For a paused or failed website download, **Build with available files** packages all selected cached pages and saved assets without more requests. Missing images, styles, or fonts are disclosed on the archive home page and in its capture report. Resume instead if you want to retry the missing downloads.

Open a failed attempt in **Activity**, choose **Delete failed attempt**, then confirm. This removes its activity entry and internal downloads, including any internal ZIM. Copies you previously exported remain in Files or Downloads. Running and paused jobs cannot be deleted through this action.

### Refresh and capture controls

In a completed collection’s **Details**, choose **Check for changes**. Review new/changed/unchanged/unavailable items and anything not seen in the new scan. Incomplete scans do not prove removal. Confirm the new selection to build a revision; exported copies remain unchanged. Website assets are revalidated during download. YouTube refresh reuses matching saved video/audio and captions. Other providers and mutable direct-file URLs may download again.

Under **Crawl & archive settings**, configure include/exclude URL globs, source asset types, a total download budget, and video quality. `*` matches any text; `?` matches one character. Rules beginning with `/` match paths, others match full URLs. Includes apply to pages; excludes apply to pages and assets. The starting page can be scanned for discovery even if not included. Budget counters survive Resume and restart for a refresh. Allow extra disk space for cached files and the output ZIM. Quality presets cap YouTube/yt-dlp resolution at 360p, 720p, or 1080p; direct files keep their original quality.

### Archive compression

In installed apps, **Settings → Archive compression** sets the Zstandard level from 1–22 (default 3). Higher levels trade build time and memory for smaller files. New jobs save this preference; resuming keeps that saved level. **Rebuild ZIM** uses the current preference and can recompress downloaded content without downloading it again. Audio, video, and images keep their existing compression. Browser and extension builds still write uncompressed ZIMs.

### Subtitles and saved-content imports

Native YouTube jobs save available captions in the original audio language by default. Choose a subtitle language (for example `en` or `pt-BR`) and whether to allow auto captions. Audio selection stays original. Captions appear in the player, detail-page transcript search, and timestamp links from collection search. Unavailable captions are reported while retaining the video. Portable collection search has a memory budget; its report discloses limits, and full per-video transcripts remain searchable on their detail pages.

On **Create**, expand **Import saved content or a URL list**. Choose files/a folder/ZIP, review the file count and size, then **Import & review**. Select the discovered items and build. Include a saved HTML page’s asset folder, or ZIP them together; ZIPs are the fallback when a device cannot select directories. Imports support stored/deflate ZIPs, with up to 50,000 files, 20 GB expanded total (or a smaller configured budget), per-file limits, 8 MB HTML and 2 MB CSS parsing limits. Encrypted archives, symbolic links, unsafe paths, and overlapping names are rejected. No missing file is fetched online. After interruption, open the attempt and **Reselect files to continue**; matching uploaded bytes are reused.

For a website URL list, paste one starting URL per line (or choose a `.txt`/`.urls` file). A single job scans within each URL’s scope. Normal review, item/depth limits, pacing, and budget apply. Media collections should use a playlist/channel/search URL.

### Capture and validation report

Completed details show captured items, saved/omitted/excluded assets, links outside the archive, and caption gaps. The ZIM’s **Capture report** gives full results in HTML and JSON. Possible empty/blocked pages are flagged for inspection. Checksum and directory checks run before the app marks a new archive ready. A validation pass does not guarantee every original page or video renders correctly. Progress also calls out prolonged inactivity; pause/resume retries from saved checkpoints.

### Static website fidelity

Saved pages activate supported reveal animations and replace supported script-dependent mobile menus with native HTML menus. Rebuilding saved content applies these repairs and refreshes readable search excerpts without re-downloading pages. CSS parsing preserves data-URL SVGs and local fragments; old erroneous stylesheet dependencies are repaired locally. Native builds can recover fonts from Google Fonts when a recognized public font proxy is temporarily unavailable, while preserving robots rules and rate-limit cooldowns.

Installed apps can **Download embedded YouTube videos** under Scope & storage. Standard YouTube/privacy-enhanced iframes and supported thumbnail players become separate, deduplicated items in review. Embedded videos count toward the item limit, always require review, and may have unknown sizes. Quality, original-audio selection, captions, file limits, total budget, pause/resume, and retry behavior use the same native downloader as YouTube collections. Completed videos play inline in the captured page and have their own detail/search entries. Unselected or unavailable videos remain clearly labeled online links. **Build with available files** omits unfinished videos rather than discarding saved pages. For older website archives, use **Check for changes**, review the discovered videos, and build a new revision; **Rebuild ZIM** alone never initiates video downloads.

Enable **Discover pages from sitemaps** in installed apps to find unlinked pages. It checks `/sitemap.xml` and same-origin sitemap declarations in robots.txt, including sitemap indexes. Discovery is checkpointed and limited to eight XML files of at most 2 MB each. The normal robots policy, backoff, download budget, page scope, URL filters, depth, and item limit still apply. Depth zero disables sitemap expansion. Unsupported, unavailable, and oversized sitemaps are logged while ordinary link discovery continues. Compressed `.xml.gz` sitemaps are not supported yet.

Both engines recover common lazy image attributes and select a bounded responsive image candidate before stripping scripts. Browser captures also preserve relative links/assets declared through HTML `base` elements. Known HTTP-200 Cloudflare verification pages are rejected with an actionable error rather than silently archived as content. Browser builds keep embedded YouTube movies as online links; the on-device extractor and sitemap discovery are currently available in installed apps. Empty supported client-rendered store catalogs get a visible notice and capture-report warning. JavaScript-only product catalogs and online forms still require the live site.
