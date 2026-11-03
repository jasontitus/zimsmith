# Android distribution and updates

[Back to Zimsmith](../README.md)

## Direct Android releases and updates

The direct-download Android flavor includes update discovery and a browser handoff. Regular `npm run android -- build ...` builds leave it disabled for store distribution; enabling a settings toggle cannot turn it on in those binaries.

```sh
# Use your own HTTPS domain or Firebase Hosting domain. Omit the environment
# variable during private testing and set the feed in Settings → App updates.
ZIMSMITH_UPDATE_FEED_URL=https://YOUR_HOST/updates/android.json npm run android:direct -- build --apk --ci
```

The installed app reads its actual Android package/version code, supported ABIs, SDK level, and signing-certificate fingerprint. A compatible feed must identify the same app, direct channel, and signing key, and offer a strictly newer version code. Automatic checks occur on opening/foregrounding and hourly while open, with a persisted daily limit. Manual checks bypass that daily interval but still honor server cooldowns. Responses are capped at 64 KB with a 15-second request timeout; redirects must stay HTTPS on the same origin. Errors keep the previous valid offer and appear in update settings. **Later** dismisses that version until a manual check or a newer release.

**Download update** opens an HTTPS URL in the external browser. It is disabled while scanning, downloading, packaging, or pausing; native code also checks before handoff. Android handles the downloaded APK and asks the user to approve installation. Zimsmith requests no app-install permission and never downloads or silently installs executable updates itself. The normal browser's “install unknown apps” approval may be needed. Keep the same package ID and production signing key across releases, and increment the Android version code. The current preview is 0.1.5 / version code 1005; older APKs without this updater need one manual update.

**Settings → App updates** has automatic/manual checks and an advanced HTTPS feed address. An empty override uses the build-time default. Changing addresses clears cached release metadata. Checks request release information only; crawl URLs, content, and job data are not sent. The host still receives ordinary HTTP request information such as the device's IP address. The app checks the feed's declared signing identity; Android verifies the actual downloaded APK signature before installing.

### GitHub Pages and Releases

The public distribution uses [GitHub Pages](https://jasontitus.github.io/zimsmith/) for the download page and update feed, and [GitHub Releases](https://github.com/jasontitus/zimsmith/releases) for immutable, checksum-named APKs. Users do not need a GitHub account to download. The static site in `site/` is deployed by `.github/workflows/pages.yml`; APKs are release assets and are not stored in Git history.

```sh
ZIMSMITH_UPDATE_FEED_URL=https://jasontitus.github.io/zimsmith/updates/android.json npm run android:direct -- build --apk --ci
node scripts/sign-android-release.mjs src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk output/builds/Zimsmith-<version>-android-arm64.apk
npm run release:android -- --apk output/builds/Zimsmith-<version>-android-arm64.apk --base-url https://jasontitus.github.io/zimsmith/ --download-base-url https://github.com/jasontitus/zimsmith/releases/download/v<version>/ --source-url https://github.com/jasontitus/zimsmith/tree/v<version> --notes-file releases/android-<version>.txt
```

Copy the generated HTML, icon, checksum, and feed from `output/hosting/` into `site/`, leaving APKs out of the site. Upload only the exact APK filename listed in `output/hosting/updates/android.json` to the matching release tag before pushing the updated site. Review release notes, run the publication audit, and verify the live feed and download after deployment. Pages uses its normal cache behavior; there is no custom server requirement or Firebase dependency.

The signing helper reads `android-release.p12` and `android-release.password` from the private user configuration directory `.config/zimsmith/signing`, or from `ZIMSMITH_SIGNING_DIR`. These files must never be committed or uploaded as release assets. Retain an offline backup of the signing key and password: future Android updates require that same key. The certificate uses the generic project identity. Public preview APKs cannot update earlier development-signed APKs in place; export archives before replacing a development installation.

Mobile release builds remap local home-directory paths in Rust and native C/C++ source locations. Before the first public upload, source, icons, and release artifacts were reviewed for private information; see [the publication review](PUBLICATION_REVIEW.md). Keep private audit reports outside the repository.

### Prepare a portable static download site

Sign the release APK with your production key first, using the Android SDK's `zipalign` and `apksigner`, or configure release signing in your build environment. Signing credentials are not stored here. Then:

```sh
npm run release:android -- --apk /absolute/path/to/signed.apk --base-url https://YOUR_HOST/ --notes-file /absolute/path/to/release-notes.txt
# Inspect output/hosting locally, then deploy to the project you choose:
firebase deploy --only hosting --project YOUR_PROJECT_ID
```

The script verifies the APK signature and inspects the actual package ID, version, minimum SDK, ABI, and certificate using Android SDK build tools. It refuses debug/development signing unless `--allow-development-key` is explicitly supplied for private testing. Output includes a branded download page, immutable checksum-named APK, SHA-256 checksum, and `updates/android.json` release feed. Re-running replaces that ABI's feed entry while retaining other ABI entries and old APKs; use the same output directory when maintaining multiple ABIs. Release notes are plain text and HTML-escaped. The generated page can also be tracked as an HTML source in [Obtainium](https://wiki.obtainium.imranr.dev/app_tracking/).

`firebase.json` serves that directory with uncached feeds and immutable APK downloads. No Firebase project, hosting domain, production signing key, or deployment is selected automatically. The output is ordinary static files and can move to any HTTPS host. Using a domain you control lets you move hosting through DNS; the advanced feed setting also allows an address change without first replacing the app.

Firebase Hosting is not a guarantee against suspension: its [applicable terms](https://firebase.google.com/terms/) include the [Google APIs Terms](https://developers.google.com/terms/), which cover third-party rights, other Google terms, and termination. Keep hosting portable rather than assuming Firebase has the same approval rules as Google Play or that it cannot remove a download site.

Validation:

```sh
cargo test -p zim-studio --lib
npx playwright test tests/android-updates.spec.ts tests/native-files.spec.ts
ZIMSMITH_TEST_APK=/absolute/path/to/development-signed.apk node --test scripts/prepare-android-release.test.mjs
```
