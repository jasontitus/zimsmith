# Live-site compatibility checks

Tested 2026-11-08 with the native engine from 0.2.11, on macOS. This is a small live-network sample, not a promise that a whole site can be archived. Site content and access rules change.

Each crawl allowed three pages, one link level, 1,000 ms page spacing, a 4 MB resource limit, a 30 MB download budget, and 150 seconds before pausing. Two different sites ran at once. Robots rules and server cooldowns stayed enabled. The standalone test worker does not render JavaScript. Sizes below use decimal MB; app budget settings use binary MB.

| Site                                                      | Saved pages | Downloaded MB | Initial result         | Final ZIM MB | Finding                                                                                                                      |
| --------------------------------------------------------- | ----------: | ------------: | ---------------------- | -----------: | ---------------------------------------------------------------------------------------------------------------------------- |
| [Ars Technica](https://arstechnica.com/)                  |           3 |         18.40 | Completed              |        16.02 | No failed or omitted assets                                                                                                  |
| [BBC News](https://www.bbc.com/news)                      |           3 |          8.07 | Failed on one asset    |         7.24 | Source HTML contains a malformed font URL returning 404; recovered without that font                                         |
| [The Guardian](https://www.theguardian.com/international) |           1 |          3.16 | Paused at time limit   |         2.06 | Large stylesheet/font dependency set; 46 assets denied by robots rules; recovery omitted 561 assets, including pending files |
| [CNN](https://www.cnn.com/)                               |           0 |          0.00 | Scan failed            |            — | Homepage exceeded the test's 4 MB per-resource cap; this is not evidence of an access block                                  |
| [NPR](https://www.npr.org/)                               |           0 |          0.00 | Scan failed            |            — | Robots request timed out; no policy bypass attempted                                                                         |
| [Rust Blog](https://blog.rust-lang.org/)                  |           3 |          1.96 | Completed              |         1.75 | No failed or omitted assets                                                                                                  |
| [Simon Willison](https://simonwillison.net/)              |           3 |         12.70 | Failed on three assets |        12.60 | Three images on a third-party host denied by robots rules; recovered without them                                            |
| [Krebs on Security](https://krebsonsecurity.com/)         |           3 |          0.86 | Paused at time limit   |         0.59 | Five assets saved, 38 pending; recovery omitted pending assets                                                               |
| [NASA](https://www.nasa.gov/)                             |           3 |         27.48 | Paused at time limit   |        26.01 | Robots exclusions and images exceeding the resource cap; recovery omitted 29 assets                                          |
| [MDN](https://developer.mozilla.org/en-US/docs/Web/)      |           3 |          1.41 | Completed              |         1.02 | No failed or omitted assets                                                                                                  |

Three sites completed normally. Five more produced ZIMs through the existing **Build with available files** operation after the crawl stopped. Recovery made zero further requests, verified by unchanged request counters. All eight outputs passed checksum and directory-structure validation. Their reports disclose omitted assets. The captured HTML and resources were readable by the ZIM library; visual fidelity and video playback were not manually verified in a reader for this matrix. A valid ZIM can still contain an incomplete page.

## What this reveals

- **Recovery matters.** A single missing font currently stops a normal build. The existing explicit recovery action works, but presenting it more clearly after nonessential asset failures would help.
- **Asset discovery overcollects.** The Guardian sample discovered hundreds of font/style resources, including alternative formats a browser would not necessarily download. Selecting appropriate font/image candidates is a useful future optimization. Greater parallelism alone does not fix this.
- **Limits need clear explanations.** CNN's overall scan error is generic; its item/log error identifies the resource cap. Surfacing that specific reason and the relevant setting would avoid confusion.
- **Crawl policy affects completeness.** Third-party image/font hosts can have different robots policies than the page. Respecting those exclusions means some archives need missing-asset reports rather than repeated retries.
- **Broader coverage is still needed.** This run did not exercise logged-in content, paywalls, full sites, native JavaScript rendering, or physical-phone playback. Deterministic tests separately cover JavaScript capture, 429 recovery, CDN concurrency, offline links, and media packaging.

## Reproduce

Use a fresh, isolated data directory; never point this test at app data containing personal crawls. Network tests are opt-in, not run on every CI push. Captures and detailed logs stay outside Git.

```sh
# Terminal 1
ZIMSTUDIO_DATA_DIR=/tmp/zimsmith-site-matrix/jobs \
ZIMSTUDIO_BIND=127.0.0.1:8793 cargo run -p zim-worker --locked

# Terminal 2, after the worker is listening
python3 scripts/test-live-sites.py --output /tmp/zimsmith-site-matrix/results.json
# Optional targeted follow-up, with the same overall/time/page limits:
python3 scripts/test-live-sites.py --site CNN --resource-mb 8 \
  --output /tmp/zimsmith-site-matrix/cnn-followup.json
```

After stopping the worker, recover any failed/paused website downloads from available files only:

```sh
cargo run -p zim-engine --example recover_site_matrix --locked -- \
  /tmp/zimsmith-site-matrix/jobs
```

The recovery utility mutates that test directory and prints validation/report summaries. It does not resume fetching. Keep it separate from an app or worker using the same data directory.
