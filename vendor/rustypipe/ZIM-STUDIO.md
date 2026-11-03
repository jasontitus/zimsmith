# RustyPipe source provenance

Upstream: <https://github.com/TeamPiped/rustypipe>

Pinned revision: `3f1491263929c22036429b665e370fdd2f066017` (2026-05-31).

This directory contains the fork's library source, manifest, README, changelog,
and license. The CLI, downloader, code generator, and repository test-data tree
are not included in the app dependency.

Local compatibility changes:

- Limit the nested workspace to the library.
- Use the non-yanked `wreq 0.15.3` line in place of the fork's yanked `wreq 5.3.0`
  dependency. Cargo.lock pins the resolved dependency graph.
- Pin the fork's original `flexon 0.4.6`; `0.4.9` fails stable Rust's borrow
  checker. This also avoids silently selecting that incompatible patch release.
- Record the upstream revision in package metadata.
- Add the current visionOS player profile from yt-dlp revision
  `b77c9304e1c4220cca450c583491904daf8455af` to `src/client/mod.rs`.
  This reuses the fork's player parser and stream extraction, with a current
  client context. The app prefers this profile when resolving media streams.

The app retains its playlist layout compatibility layer in
`crates/engine/src/youtube_playlist.rs`. New upstream snapshots must pass the
app's regression tests, live metadata/stream checks, and mobile builds before
replacing this one. This is a vendored fork snapshot, not the crates.io release.

Upstream source formatting is retained. Use `npm run format:rust` and
`npm run format:rust:check` to format the app workspace without rewriting
the vendored dependency.
