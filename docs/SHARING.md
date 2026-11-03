# Share a link to Zimsmith

On Android, choose **Share → Zimsmith** in your browser or YouTube app. Zimsmith opens with the link and source type filled in. Tap **Scan shared link**, or adjust the settings first.

On iOS, choose **Share → Zimsmith → Save link**, then open Zimsmith. Tap **Scan shared link** to start, or adjust the crawl form first. You may need to enable Zimsmith under **More** in the share sheet the first time. Apple's Share Extension API does not support automatically opening its containing app.

Shared links remain in a private inbox until a crawl is successfully created or you dismiss them. A new share does not replace a form you are already editing; use **Set up crawl** to load it. Sharing does not start downloads automatically.

## Building

Android's receiver is included in the mobile-files plugin manifest. iOS requires XcodeGen and an Apple development team supporting App Groups. The mobile build wrapper configures the extension, embeds it, and gives both targets `group.<app identifier>`. Enable that App Group for both app identifiers in your signing setup if automatic provisioning cannot do so. Generated Xcode projects and signing profiles stay outside Git.

Run the interface tests with `npx playwright test tests/shared-links.spec.ts` while the development UI is running. CI also checks URL validation, compiles the extension, and tests persistence and acknowledgement in the shared Swift inbox.
