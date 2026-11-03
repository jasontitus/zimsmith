# MD5 MSVC compatibility patch

Vendored RustCrypto `md-5` 0.10.6 from the crates.io package, with its original MIT/Apache-2.0 licenses and source retained. Upstream repository: https://github.com/RustCrypto/hashes

Upstream revision: `026b0e81e90ba476a5d343a8e4efbc63b814737b`.

The pinned ZIM writer enables the `asm` feature. `md5-asm` 0.5.2 contains GNU-style assembly that cannot be built by the Windows MSVC toolchain. This patch excludes that optional dependency and assembly dispatch on MSVC, using the crate's existing portable Rust implementation instead. Other targets keep the original selection. No MD5 algorithm code is changed.

The upstream test vectors are retained and run in Windows CI with the `asm` feature enabled, verifying this fallback under the same feature request used by the ZIM writer.
