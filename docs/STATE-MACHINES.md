# Zimsmith state machines

Audited 2026-11-08 against `bab67d1` and the fixes described below. State names are code values. This is a source and regression-test audit, not a formal proof of every OS callback or storage failure. Native and browser engines have separate implementations. Tables describe their shared contract and call out differences.

## Confirmed issues, ordered by impact

### SM-1 · High · Exclusive ownership during imports and browser commands

Tracked in [GitHub issue #1](https://github.com/jasontitus/zimsmith/issues/1).

**Impact.** Concurrent commands can change configuration or uploaded files while a finalizer reads them. Browser refresh and rebuild can operate on the same collection simultaneously, causing conflicting phases, revisions, and checkpoints.

**Native evidence.** `Engine::begin_import` accepted `importing` and paused unfinished imports without checking `Record.running`. `import_chunk` checked `running` before waiting for the job mutex. A finalizer could claim ownership during that wait, then the chunk would append while preparation was underway. `finish_import` claimed ownership without first locking and validating the job. Pause followed by reselect could therefore alter the finalizer's input/configuration before it unwound. Sources are `crates/engine/src/imports.rs`, `begin_import`, `import_chunk`, and `finish_import`.

**Browser evidence.** Worker message callbacks await independently in `src/browser/worker.ts`. In `BrowserEngine.dispatch`, refresh awaits directory access and fingerprints while status remains `completed`. A second refresh or rebuild passed `idle` and mutated the same object. Paused imports could also pass `idle` while their aborted preparation still held an entry in `controls`. JavaScript's single thread does not prevent interleaving at these awaits.

**Fixed.** Native finalizer reservation and upload guards now share the job mutex. Reselection rejects a still-owned import. Browser commands reserve the job before their first await and release ownership in `finally`. `idle` checks the live controller as well as status. Pause and cancel remain callable while preparation is running.

**Regression evidence.** `import_commands_preserve_finalizer_ownership_and_paused_state` queues an upload behind the checkpoint mutex, assigns finalizer ownership, and verifies rejection without appended bytes. It also covers pause/reselection and successful subsequent finalization. `tests/state-machines.spec.ts` suspends refresh at directory access and attempts concurrent refresh/rebuild, checks unchanged revision and status, injects a storage error, and verifies that another retry reaches storage. A separate test verifies a paused job with a surviving controller rejects resume.

### SM-2 · Medium · Reject lifecycle commands outside their source states

Tracked in [GitHub issue #2](https://github.com/jasontitus/zimsmith/issues/2).

**Impact.** Browser clients could approve an already completed archive again or rebuild an imported collection before its review was confirmed.

**Evidence.** Browser `build_job` previously checked only idle status and item availability. `rebuild_job` checked downloaded entries and template, with no phase/status guard. Imported entries are already `done` at review, so this bypass was reachable through the command API. These are command-level defects even where the normal screen hides the action.

**Fixed.** Browser approval accepts review/failed jobs. Rebuild requires package phase and completed/failed/paused status. Native finish validates an unfinished importing job before claiming ownership or entering failure handling.

**Regression evidence.** `tests/state-machines.spec.ts` rejects approval from completed and rebuilding from scan/review. The native import regression verifies invalid finish calls preserve paused/review status, followed by a successful reselect/upload/finalize sequence.

## Job lifecycle

The explicit state is `Job.status` plus `Job.phase`. Implicit state includes selection, import completion, checkpoints, output revision and worker ownership. See `crates/engine/src/lib.rs`, `model.rs`, `imports.rs`, `refresh.rs`, and `src/browser/engine.ts`.

| Phase                  | Permitted stable statuses            | Required interpretation                                                                               |
| ---------------------- | ------------------------------------ | ----------------------------------------------------------------------------------------------------- |
| `import`               | importing, paused, failed            | Approved file list exists while upload/preparation is unfinished                                      |
| `scan`                 | scanning, review, paused, failed     | Discovery queue and visited set determine work remaining; completed imports enter review here         |
| `download`             | downloading, paused, failed          | Selection has been approved; cached items may already be done                                         |
| `package`              | packaging, completed, paused, failed | Selected inputs have finished downloading or were explicitly accepted through available-file recovery |
| Any active crawl phase | pausing                              | Cancellation requested; owner may still hold files                                                    |
| Tombstoned native job  | deleted                              | Removed from engine map; no retry permitted                                                           |

A visible terminal or paused status alone does not prove the worker released ownership. Commands must also check `running` (native), `controls` and command reservations (browser). Browser active statuses can mean queued work, because its drain runs one crawl at a time. Native jobs run independently with shared host pacing.

| From                                | Trigger and guard                                 | To                            | Checkpoint/retry behavior                                                      |
| ----------------------------------- | ------------------------------------------------- | ----------------------------- | ------------------------------------------------------------------------------ |
| Absent                              | Create valid source                               | scan/scanning                 | Persist before launching                                                       |
| Absent                              | Begin approved import                             | import/importing              | Establish upload manifest                                                      |
| scan/scanning                       | Successful scan, complete eligible small job      | download/downloading          | Persist automatic approval decision                                            |
| scan/scanning                       | Other successful scan                             | scan/review                   | Explicit approval required                                                     |
| scan/review or failed               | Build with nonempty available selection           | download/downloading          | Native imported files go directly to package                                   |
| Active                              | Pause                                             | pausing → paused              | Drop work, retain resumable inputs; imports mark paused and cancel preparation |
| Active                              | Operation error                                   | same phase/failed             | Persist reason and retain checkpoints                                          |
| paused or failed                    | Resume, owner released, import finished or absent | active status for saved phase | Native resets retryable entry/resource errors; server cooldown persists        |
| import/paused or failed             | Reselect matching files, owner released           | import/importing              | Resume at saved upload offsets                                                 |
| import/importing                    | Finish, valid uploaded lengths                    | scan/review                   | Save entries/resources and finished flag, then clean upload scratch files      |
| download/downloading                | Selected downloads finish                         | package/packaging             | Cached payloads become packaging inputs                                        |
| download/paused or failed           | Native Build with available files                 | package/packaging             | Require cached pages, omit unavailable media/assets and report omissions       |
| package/packaging                   | Writer finishes and validation passes             | package/completed             | Publish output and size                                                        |
| package/completed, failed or paused | Rebuild with all selected inputs done             | package/packaging             | Increment revision; reuse cached content                                       |
| completed                           | Refresh                                           | scan/scanning                 | Preserve output, snapshot prior entries, disable auto approval                 |
| stopped failed/paused               | Edit settings                                     | scan or saved phase/active    | Discovery-affecting edits rescan; other edits resume                           |
| completed/failed/paused media       | Native original audio repair after scan           | download/downloading          | Preserve approved selection, re-resolve original track                         |
| Active checkpoint on startup        | Recovery                                          | same phase/paused             | Never silently restart downloads                                               |
| failed and stopped                  | Delete                                            | absent                        | Native rename into `.deleted`; browser delete manifest before files            |

Native edit also accepts review. Browser edit currently accepts failed/paused only. Imports require reselection while unfinished and reject ordinary resume. Native deletion may return deferred disk cleanup; browser orphan cleanup likewise retries at startup.

## WARC import preparation (added 2026-11-08)

Both engines expand WARC/WARC.GZ inside the existing unfinished-import owner reservation. Native record decoding runs in a blocking worker with cancellation checks; the browser worker checks its abort signal between stream chunks. Payloads are staged before the job becomes reviewable. Reading local captures never schedules network requests.

Native imports count up to 200,000 physical records per file. The native importer retains every usable capture within its storage and record guards. The crawl item limit does not truncate WARC review. Review uses paginated rows and the full selection, and the app sends compact list updates separately from item details. A malformed captured HTTP header or invalid target URL skips only that record. Explicit WARC truncation markers always omit the marked record. WARC framing errors still fail preparation and leave the uploaded file available for retry.

The Common Crawl regression was reproduced on the 1,135 MB CC-MAIN-2024-10 shard `CC-MAIN-20240220211055-20240221001055-00000.warc.gz`. The old importer failed with “invalid header name” at record 7,686. The first recovery fix scanned 104,995 records and built a validated ZIM from a 100-item selection with zero network requests. The full native import of the same shard produced a 604.09 MB ZIM containing 33,835 page and media items and 439 captured assets. It passed checksum and directory-structure validation with zero network requests. The capture stayed outside the repository. [Common Crawl archive announcement](https://www.commoncrawl.org/blog/february-march-2024-crawl-archive-now-available)

| From                                                         | Event/guard                                 | To                                     | Invariant                                                                                            |
| ------------------------------------------------------------ | ------------------------------------------- | -------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| import/importing, uploaded lengths match                     | Finish reserves owner                       | import/importing, reading records      | No concurrent upload, reselect or second finalizer                                                   |
| Reading header/body                                          | Valid complete response/resource            | Next record                            | Bounded headers, encoded stream, decoded body, record count and storage; original URL/MIME retained  |
| Reading records                                              | Same URL captured again                     | Replace staged payload                 | Last complete record in input order wins; no mixed old/new body                                      |
| Reading records                                              | Unsupported revisit/redirect/partial record | Next record + omission count           | Never present omitted body as a complete saved page                                                  |
| Reading/preparing                                            | Malformed input or size/storage failure     | import/failed                          | Import stays unfinished; ownership released; uploaded source retained                                |
| Reading/preparing                                            | Pause/cancel                                | import/paused or failed; owner unwinds | Reselection waits for owner release; uploaded offsets retained                                       |
| import/paused or failed, owner released                      | Reselect same paths/sizes                   | import/importing                       | Limits may be changed; extraction restarts from source rather than resuming a partial decoded record |
| All records prepared + at least one page/media + limits pass | Finish commit                               | scan/review                            | Finished flag and entries/resources published together; explicit review remains required             |

`crates/engine/tests/warc_import.rs` verifies plain/multi-member compressed captures through validated ZIM packaging, URL/asset rewriting, duplicate replacement, omission warnings, malformed input, decoded limits, larger containers and retry with retained uploads. `tests/warc-import.spec.ts` exercises the browser dispatcher/parser; `tests/browser-imports.spec.ts` imports a compressed WARC through the real UI/worker and independently validates its exported ZIM, with no source-network requests. The WARC-specific browser test also pauses during preparation, rejects reselect while ownership is held, then retries using the retained upload. Existing native ownership regression tests apply to WARC through the same import finalizer. Local 0.2.12 validation passed 92 native tests (6 intentionally ignored) and 33 browser/extension tests.

## Entries, assets, and approval

| Machine                  | States                                                  | Transitions and invariants                                                                                                                                                                                                                                                               |
| ------------------------ | ------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Native page/media entry  | pending, scan_failed, failed, done; selected boolean    | Scan success → pending; failed discovery → scan_failed; download success → done; retry failed → pending. Resume of a failed native website scan removes failed entries and requeues their URLs. A scan_failed item cannot be approved                                                    |
| Native linked resource   | pending, done, failed, excluded, skipped                | Fetch success → done. Capture settings → excluded. Robots denial or HTTP 404/410 → skipped with a reported omission. Other errors → failed. Resume retries failed resources, and refresh reevaluates skipped resources. Available-file recovery turns unfinished resources into skipped. |
| Browser entry            | pending, done, error-bearing checkpoint                 | Failed requests throw while their queue head remains pending; resume reattempts the same request. A page must have a saved file before approval                                                                                                                                          |
| Browser resource receipt | absent, partial, complete, omitted, revalidate          | Interrupted writes retain offsets. Rebuild reuses cached receipts and robots/404/410 omissions. Refresh clears omissions and marks resources for conditional revalidation.                                                                                                               |
| Approval                 | unreviewed, review, approved                            | Incomplete scans, refreshes, dynamic native captures and embedded video discoveries require review. Auto-start depends on saved threshold and complete error-free selection. Import preparation alone never approves a build                                                             |
| Refresh comparison       | prior snapshot, comparing, compared complete/incomplete | Compare URLs/fingerprints and preserve selection. Missing entries are `not_seen`; incomplete discovery cannot establish removal                                                                                                                                                          |
| Media tracks             | unresolved, selected original, partial, complete, muxed | Verify original language before quality selection. Track identity participates in cache identity; separate audio/video combine before publishing. Repair returns selected media to pending                                                                                               |
| Captions/artwork         | unknown, fetching, saved, unavailable                   | Optional failures are disclosed; saved text/artwork is reused. Subtitle policy controls reuse                                                                                                                                                                                            |

Relevant sources include `web.rs`, `native_media.rs`, `youtube_audio.rs`, `media.rs`, `captions.rs`, `thumbnails.rs`, `refresh.rs`, `src/browser/network.ts`, and `imports.ts`.

## Request scheduling, policy and cancellation

| Machine               | States                                                      | Transition rules and invariants                                                                                                                                                                                                   |
| --------------------- | ----------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Host pacing           | ready, ordinary spacing, cooldown                           | Requests wait for host timing; 429/503 and retry headers extend cooldown. Editing ordinary delay does not erase server backoff                                                                                                    |
| Robots policy         | absent/expired, fetching, allowed/denied, unavailable       | Cache origin-bound policy; expiry permits refresh; explicit denial excludes the resource. Narrow passive CDN fallback does not override explicit denials                                                                          |
| CDN scheduler         | serial, rolling pool, throttled serial                      | Only eligible linked passive assets enter configured 1–6 pool. A completion frees a slot. Declared crawl delay, strict total budgets and ordinary origins stay serial. Throttling cancels the pool and retains queued checkpoints |
| Asset request counter | zero, active N, peak N                                      | RAII increments at send and decrements on completion/drop. Peak records actual simultaneous requests, independently of completed file count                                                                                       |
| Native runner         | unowned, owned, cancel requested, releasing                 | Atomic reservation permits one driver; launch registers stop notification and checks pausing before drive. Release follows checkpoint/error handling                                                                              |
| Browser runner        | queued, controller owned, abort requested, released         | One drain; controller removed in finally. Per-job mutation reservation covers asynchronous command preparation. Read and pause/cancel commands remain responsive                                                                  |
| Import upload         | approved, partially uploaded, uploaded, preparing, reviewed | Append only at exact current offset within approved length. Preparation owns input files until unwind. Cancellation does not grant a second preparer ownership                                                                    |

Sources are `web.rs`, `robots_cache.rs`, `lib.rs`, `imports.rs`, `src/browser/network.ts`, `engine.ts`, and `storage.ts`. Backoff is bounded, persisted where applicable, and respected on retry. Native host semaphores and browser Web Locks address different scopes of concurrency. Web Locks prevent two browser workspace owners; they do not serialize messages inside one worker.

## Rendering, packaging and file publication

| Machine               | States                                                                  | Transitions and invariants                                                                                                                                                                         |
| --------------------- | ----------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Page loading decision | static, automatic detection, render required                            | Automatic mode inspects missing content; rendering needs platform support and compatible budget. Unavailable/incomplete rendering prevents automatic approval                                      |
| Platform renderer     | waiting, started, polling, settled/incomplete, failed/canceled          | One platform page browser; core has queue and execution deadlines. Platform callbacks carry request identity and completion guards; cancellation drops the rendering future and closes its browser |
| Capture script        | loading, waiting on requests/DOM, scrolling/paginating, snapshot        | Bounded time, scroll and pagination loops; pending counters settle before normal snapshot; rate limit produces an incomplete result and cooldown                                                   |
| Native writer         | cached inputs, rewrite/mux/index, partial archive, validated, published | Stream bounded data, validate directory/checksum, rename partial archive before completed checkpoint. Restart packaging from cached inputs after cancellation                                      |
| Browser writer        | cached inputs, rewritten items, writing revision, validated, published  | Check storage quota and abort signal, close sync handle in finally, expose output only after validation. Rebuild increments revision                                                               |
| Export                | unavailable, ready, picker pending, saved/canceled/error                | Native artifact requires completed; browser additionally verifies saved file length. iOS pending-save slot admits one picker and clears on completion/cancel                                       |

Sources are `render.rs`, `src-tauri/src/rendering.rs`, `crates/mobile-files/{ios,android}`, `crates/engine/assets/render.js`, `archive.rs`, `mux.rs`, `mp4_layout.rs`, `src/browser/zim.ts`, and `engine.ts`. Publication spans file and manifest operations rather than one filesystem transaction. Existing successful outputs and validation metadata must be interpreted with status/revision, especially during a rebuild.

## UI, shared links, updates and offline players

| Machine             | States                                                                  | Transitions and invariants                                                                                                                                |
| ------------------- | ----------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| App navigation/form | create, activity/details, settings; new/edit/import/shared-link draft   | Selecting job updates detail/selection; submission sets busy; response clears busy and displays result/error. Cancel editing restores creation flow       |
| Connection          | connecting, connected, disconnected                                     | Poll API success/failure; polling continues after failure                                                                                                 |
| Review selection    | job-derived, user-edited, submitted                                     | Preserve user selection during stable review; reset when selected job or lifecycle changes                                                                |
| Shared URL inbox    | received, persisted, offered, draft, acknowledged                       | Persist link before app handoff; normalize source; populating a draft does not start a crawl. Busy/occupied forms defer automatic adoption                |
| Android updater     | unavailable/unconfigured, idle, checking, cooldown, available/dismissed | Serialize preferences/checks; persist attempt before network; reject invalid release/feed URLs; installation handoff opens browser after active-job check |
| Gallery/player      | closed, loading, playing, paused/ended/error                            | Thumbnail opens player; title opens detail; close stops/removes current media; Next follows visible order. Stale events check current-player identity     |
| Saved-page preview  | cover visible, play requested, playing, stopped                         | Local image cover starts native media on user gesture and hides on playback; archive CSP permits only the bundled helper                                  |

Sources are `src/App.tsx`, `src/SavedContentImport.tsx`, `src/SharedLinks.tsx`, `src/AndroidUpdates.tsx`, `src-tauri/src/updates.rs`, `src-tauri/share-extension`, `crates/mobile-files/ios/Sources/SharedURLInbox.swift`, and `crates/engine/assets/{gallery,media}.js`.

## Validation and limits

The fixes are covered by deterministic engine-level tests with injected storage suspension and a controlled native lock interleaving. Native imports are also exercised through successful upload/finalize after retry. Browser tests use the real command dispatcher with a minimal store fixture; platform WebViews and physical devices are outside those tests.

The browser scan-retry suspicion was ruled out by inspecting its queue handling. It shifts the queue only after success or an intentional robots exclusion. No bug is claimed there. Existing tests cover native recovery, refresh, import offsets, packaging validation, rate limits, CDN concurrency, rendering cancellation and media repair. OS termination and storage-device failure at every persistence boundary have not been exhaustively fault-injected.

Verification on 2026-11-08 passed with the fixes applied.

- Native suite, 85 passed and 6 intentionally ignored live/external-fixture tests.
- Browser state regressions, 3 passed. All three failed against the original engine.
- Native import ownership regression failed against the original implementation at the queued-upload assertion and passed with the fix.
- Browser saved-ZIP import, review, export and ZIM validation integration test passed.
- Desktop frontend and browser production builds, scoped Rust formatting and diff checks passed.
- The browser regression file is included in the existing `test:browser` CI command.

GitHub CI passed for implementation commit `628b5122c78db79703d9b2de144f06ad357fb0ee`. The completed [Check run](https://github.com/jasontitus/zimsmith/actions/runs/42462348451) verified the following coverage.

| Platform                | Verified result                                                                                                                                                                               |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Linux, macOS, Windows   | 85 native engine/worker tests passed on each platform, with 6 intentionally ignored tests. Frontend build, Tauri compile check, sharing test, and share-extension synchronization test passed |
| Linux browser/extension | All 24 tests passed, including the state-machine regressions, imports, captions, snapshots, embedded playback, JavaScript capture, and website templates                                      |
| iOS                     | ARM64 engine compile check, share-extension typecheck, and durable-inbox test passed                                                                                                          |
| Windows MD5             | Portable backend checks passed with the requested `asm` feature                                                                                                                               |

These CI checks cover compilation and automated fixtures. They do not establish physical-device interruption behavior or an Android build for this change.

Evidence lives in [native workflow regressions](../crates/engine/tests/workflow.rs), [browser state regressions](../tests/state-machines.spec.ts), [native import implementation](../crates/engine/src/imports.rs), and [browser command implementation](../src/browser/engine.ts).
