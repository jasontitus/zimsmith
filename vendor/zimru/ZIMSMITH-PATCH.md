# ZIM library source provenance

Library source snapshot of https://github.com/jasontitus/zimru at revision `061afbc2920c78fc4ecb7211cb9ef4f3655dba35`. Original MIT license retained.

Local changes include restricting the `memmap2::Advice` import and two optional memory-map advice calls to Unix targets. Windows retains mapping, bounds checks, page touching, checksum verification, and archive writing. The writer also supports serial encoding and immediate publication of streamed clusters to the partial ZIM. Zimsmith uses this mode on iOS and when replacing an interrupted partial file.

Library source, manifest, build script, and headers are included. Unused CLI and benchmark target declarations are removed from the manifest. The upstream repository's CLI, benchmark, integration-test fixture tree, and development documents are not bundled in this app dependency. The application's archive and recovery tests exercise the library on the supported build platforms.
