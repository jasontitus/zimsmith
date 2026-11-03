# Zimsmith identity

Zimsmith makes offline collections from websites and media. Its folded-paper Z and stacked pages represent making and preserving ZIM archives.

The source icon is [`zimsmith-icon.png`](zimsmith-icon.png). Generated with the built-in image generation tool, then exported to platform icon sizes with `tauri icon`. iOS and Android display **Zimsmith**; the existing bundle identifier and data location are retained for upgrades.

## Exact generation prompt

Use case: logo-brand. Asset type: production app icon for Zimsmith, an on-device website/video crawler and ZIM archive creator. Primary request: design one original, distinctive icon: a bold folded-paper ribbon forming an abstract Z, with two subtle stacked-page edges suggesting a collection being preserved. Style: exceptionally clean premium app icon, crisp geometric silhouette, restrained tactile paper depth, warm ivory main folded ribbon and a small apricot fold edge on a rich violet-indigo full-bleed background, matching a violet productivity app. Composition: square 1024x1024 flat front view, one large centered mark inside the central 68 percent safe area, balanced negative space; edges perfectly sharp at small sizes, no tiny details. Background must fill the ENTIRE square, solid opaque corners, no baked-in rounded-square outer mask, no external frame. The icon itself is the artwork, not a mockup or a screenshot. No printed text, wordmarks, watermarks, badges, drop shadows outside the artwork, extra symbols, devices, or surrounding objects. It should feel useful, calm, and unmistakable on an iPhone home screen.

## Regenerate platform exports

```sh
npx tauri icon assets/branding/icons.json --output src-tauri/icons
npx tauri icon assets/branding/zimsmith-icon.png --output assets/branding/sizes --png 16 --png 32 --png 48 --png 128 --png 256
```

The smaller exports supply the app UI, browser favicon, and ZIM illustration. Mobile icon exports are also copied into the generated Xcode and Android resources before building.
