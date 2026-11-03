# Public publication review

Reviewed before the first public upload on 2026-11-02.

The publication tree contains application source, required vendored library source and licenses, bundled assets, tests, build scripts, documentation, and the static download site. It excludes crawl caches, downloaded content, device logs, screenshots/traces, generated mobile projects, local tooling checkouts, build outputs, credentials, signing keys, and provisioning files.

Checks performed:

- Gitleaks over a separate copy of the exact publication tree, with redacted findings.
- Additional inspection for email addresses, local home-directory paths, private key/token patterns, device identifiers, and personal signing/team information.
- Image metadata inspection for author, owner, comments, location, and camera identifiers; no identifying image metadata found in project branding/icons.
- Recursive inspection of release ZIP/APK members for local paths, private keys, credentials, and known private device/signing identifiers.
- The user approved GPL-3.0 publication before source or release uploads. The original project is marked GPL-3.0-only; upstream licenses and attribution remain intact.
- Local Git commits use a project name and GitHub no-reply email; personal email addresses are not used for commit attribution.
- Signing keys and passwords stay outside the repository. Public certificates use a generic project identity. Certificate fingerprints in release metadata are intentionally public.

Findings and resolutions:

- A compiler state file was excluded. Generated mobile projects and other machine-specific output remain excluded.
- An earlier private Android build embedded local home-directory paths. Public builds remap Rust and native C/C++ source paths before compilation; the rebuilt APK is scanned again before upload.
- Gitleaks flags two fixed pagination-token assertions in the vendored RustyPipe channel tests. These are public, deterministic request-pagination fixtures, not authentication credentials. The findings were manually reviewed without blanket exclusions for vendored code.
- Email-like strings in first-party source are URL-validation test fixtures or a CSP directive. Vendored author attribution and public provider/channel-description fixtures were retained as upstream material.
- A follow-up Windows build patch vendors RustCrypto md-5 0.10.6 with its original licenses and upstream vectors. The added source is reviewed with the same PII and credential checks before upload; only platform/backend selection is changed.
- The matching MIT-licensed ZIM library snapshot is retained with three Unix-only hint guards for Windows. Only library source and build/header files are included; unrelated upstream tools, fixtures, and developer documents are excluded. This source snapshot is scanned before upload.
- Public GitHub account/repository URLs are intentional publishing and dependency addresses. No private device identifiers or personal signing-team identifiers are included.

The 0.1.4 audio-selection repair includes a generated sine-wave audio fixture. Live verification against a user-supplied video keeps the URL, track report, and downloaded media outside the publication tree. The source, fixture, APK, and ZIPs receive the same checks before upload.

Recheck future releases:

```sh
python3 scripts/audit-publication.py --artifact /path/to/release.apk --report /private/tmp/publication-audit.json
gitleaks dir /path/to/exported-staged-tree --redact --report-format json --report-path /private/tmp/gitleaks.json
```

Inspect any findings before publishing. A clean automated scan is evidence for the reviewed files, not a guarantee that a detector can recognize every kind of sensitive information. Private reports and scan configuration are not published.
