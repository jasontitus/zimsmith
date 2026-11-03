//! The same renderer supplies the exported archive and the Settings preview.
use crate::{archive::escape, ArchiveTemplate, Entry, NewJob, SourceKind};
use base64::Engine as _;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const ICON: &[u8] = include_bytes!("../assets/zimsmith-icon-48.png");
pub const CSS: &str = include_str!("../assets/archive.css");
pub const GALLERY_JS: &str = include_str!("../assets/gallery.js");
pub const SEARCH_JS: &str = include_str!("../assets/search.js");
pub const SANS: &[u8] = include_bytes!("../assets/fonts/dm-sans.woff2");
pub const DISPLAY: &[u8] = include_bytes!("../assets/fonts/manrope.woff2");
pub const FONT_LICENSE: &str = include_str!("../assets/fonts/LICENSE.txt");

#[derive(Serialize)]
pub struct TemplateOption {
    pub id: ArchiveTemplate,
    pub name: &'static str,
    pub description: &'static str,
}

pub fn catalog() -> Vec<TemplateOption> {
    vec![
        TemplateOption {
            id: ArchiveTemplate::Atlas,
            name: "Atlas",
            description: "A bright, spacious library with violet accents and a card grid.",
        },
        TemplateOption {
            id: ArchiveTemplate::FieldNotes,
            name: "Field Notes",
            description: "Warm paper, serif headings, and an editorial reading list.",
        },
        TemplateOption {
            id: ArchiveTemplate::Midnight,
            name: "Midnight",
            description: "A dark collection with cool mint accents and crisp typography.",
        },
        TemplateOption {
            id: ArchiveTemplate::ScreeningRoom,
            name: "Screening Room",
            description:
                "A video gallery with offline thumbnails, duration badges, and a cinema player.",
        },
    ]
}

#[derive(Serialize)]
pub struct Document {
    pub title: String,
    pub url: String,
    pub excerpt: String,
    pub kind: &'static str,
    pub detail: String,
    pub thumbnail: Option<String>,
    pub media: Option<String>,
}

#[derive(Default, Serialize)]
pub struct SearchCatalog {
    pub omitted_assets: usize,
    pub documents: Vec<Document>,
    pub index: BTreeMap<String, Vec<usize>>,
}

impl SearchCatalog {
    pub fn add(&mut self, entry: &Entry, kind: &SourceKind) {
        let tokens: BTreeSet<_> = format!("{} {}", entry.title, entry.text)
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        for token in tokens {
            self.index
                .entry(token)
                .or_default()
                .push(self.documents.len());
        }
        let text = entry.text.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut excerpt: String = text.chars().take(160).collect();
        if text.chars().count() > 160 {
            excerpt.push('…');
        }
        let media = matches!(kind, SourceKind::Media);
        let audio = entry.media_file.as_deref().is_some_and(|file| {
            [".mp3", ".m4a", ".ogg", ".wav"]
                .iter()
                .any(|ext| file.ends_with(ext))
        });
        if excerpt.is_empty() {
            excerpt = if media {
                "Saved for a little more discovery, wherever you go."
            } else {
                "A page from your collection, ready to read offline."
            }
            .into();
        }
        let detail = if media {
            entry
                .duration
                .filter(|n| n.is_finite() && *n >= 0.0)
                .map(|n| {
                    let s = n as u64;
                    if s >= 3600 {
                        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
                    } else {
                        format!("{}:{:02}", s / 60, s % 60)
                    }
                })
                .unwrap_or_else(|| "Offline playback".into())
        } else {
            let words = entry.text.split_whitespace().count();
            format!("{} min read", words.div_ceil(220).max(1))
        };
        self.documents.push(Document {
            thumbnail: entry.thumbnail_file.clone(),
            media: media.then(|| entry.media_file.clone()).flatten(),
            title: entry.title.clone(),
            url: format!("{}.html", entry.id),
            excerpt,
            kind: if audio {
                "Audio"
            } else if media {
                "Video"
            } else {
                "Article"
            },
            detail,
        });
    }
}

fn card(document: &Document, number: usize, preview: bool, gallery: bool) -> String {
    let symbol = match document.kind {
        "Video" => "▶",
        "Audio" => "♪",
        _ => "↗",
    };
    let href = if preview {
        "#collection"
    } else {
        &document.url
    };
    if gallery {
        let image = document.thumbnail.as_ref().map(|file| format!("<img src='{}' alt='' loading='lazy' decoding='async' width='640' height='360'>", escape(file))).unwrap_or_default();
        let play = if preview {
            String::new()
        } else {
            document
                .media
                .as_ref()
                .map(|file| {
                    format!(
                        " data-play-src='{}' data-play-kind='{}' data-play-title='{}'",
                        escape(file),
                        document.kind,
                        escape(&document.title)
                    )
                })
                .unwrap_or_default()
        };
        let (thumbnail_start, thumbnail_end) = if document.media.is_some() {
            (
                format!(
                    "<button type='button' class='video-thumbnail'{play}{}",
                    if preview { " disabled" } else { "" }
                ),
                "button",
            )
        } else {
            (
                format!("<a class='video-thumbnail' href='{}'", escape(href)),
                "a",
            )
        };
        let action = if document.media.is_some() {
            "Play"
        } else {
            "Open"
        };
        return format!(
            r#"<li class="archive-item" data-document="{number}"><article class="gallery-card">{thumbnail_start} aria-label="{} {}"><span class="video-art artwork-{}">{image}<span class="artwork-number" aria-hidden="true">{:02}</span><span class="video-play" aria-hidden="true">{symbol}</span><span class="video-duration">{}</span></span></{thumbnail_end}><a class="entry-detail" href="{}"><span class="entry-copy"><span class="entry-meta"><span class="entry-kind">{}</span><span>AVAILABLE OFFLINE</span></span><h3>{}</h3><span class="entry-excerpt">{}</span><span class="entry-bottom"><span>More info</span><span aria-hidden="true">↗</span></span></span></a></article></li>"#,
            action,
            escape(&document.title),
            number % 4,
            number + 1,
            escape(&document.detail),
            escape(href),
            document.kind,
            escape(&document.title),
            escape(&document.excerpt)
        );
    }

    format!(
        r#"<li class="archive-item" data-document="{number}"><a class="entry-link" href="{}"><span class="entry-number" aria-hidden="true">{:02}</span><span class="entry-copy"><span class="entry-meta"><span class="entry-kind">{}</span><span>{}</span></span><h3>{}</h3><span class="entry-excerpt">{}</span><span class="entry-bottom"><span>Open {}</span><span class="entry-symbol" aria-hidden="true">{symbol}</span></span></span></a></li>"#,
        escape(href),
        number + 1,
        document.kind,
        escape(&document.detail),
        escape(&document.title),
        escape(&document.excerpt),
        document.kind.to_lowercase()
    )
}

pub fn render_index(
    template: ArchiveTemplate,
    config: &NewJob,
    catalog: &SearchCatalog,
    preview: bool,
) -> anyhow::Result<String> {
    let name = match template {
        ArchiveTemplate::Atlas => "A little more discovery.",
        ArchiveTemplate::FieldNotes => "Collected for the curious.",
        ArchiveTemplate::Midnight => "Keep exploring. Anywhere.",
        ArchiveTemplate::ScreeningRoom => "Your front row. Anywhere.",
    };
    let cards: String = catalog
        .documents
        .iter()
        .enumerate()
        .map(|(i, doc)| card(doc, i, preview, template == ArchiveTemplate::ScreeningRoom))
        .collect();
    let json = serde_json::to_string(catalog)?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    let source = if preview { "#collection" } else { &config.url };
    let sample = if preview {
        "<span class='preview-label'>Sample collection · style preview</span>"
    } else {
        ""
    };
    let (assets, policy) = if preview {
        let encoded = base64::engine::general_purpose::STANDARD;
        let css = CSS
            .replace(
                "url(\"archive-sans.woff2\")",
                &format!("url(\"data:font/woff2;base64,{}\")", encoded.encode(SANS)),
            )
            .replace(
                "url(\"archive-display.woff2\")",
                &format!(
                    "url(\"data:font/woff2;base64,{}\")",
                    encoded.encode(DISPLAY)
                ),
            );
        (format!("<style>{css}</style>"), "default-src 'none'; style-src 'unsafe-inline'; font-src data:; img-src data:; base-uri 'none'; form-action 'none'")
    } else {
        ("<link rel='stylesheet' href='studio.css'>".into(), "default-src 'none'; script-src 'self'; style-src 'self'; font-src 'self'; img-src 'self' data:; media-src 'self'; base-uri 'none'; form-action 'none'")
    };
    let scripts = if preview {
        String::new()
    } else {
        let gallery = if template == ArchiveTemplate::ScreeningRoom {
            "<script src='gallery.js'></script>"
        } else {
            ""
        };
        format!("<script id='catalog' type='application/json'>{json}</script><script src='search.js'></script>{gallery}")
    };
    let collection_heading =
        if template == ArchiveTemplate::ScreeningRoom && config.kind == SourceKind::Media {
            "Pick something to watch."
        } else {
            "Something worth keeping."
        };
    let capture_notice = if catalog.omitted_assets > 0 {
        format!("<aside class='archive-noscript' aria-label='Incomplete capture'><strong>Built with available files.</strong> {} assets are missing. Some images, styles, or fonts may be absent. <a href='capture-report.html'>View capture report</a></aside>", catalog.omitted_assets)
    } else {
        String::new()
    };
    let preview_class = if preview { " archive-preview" } else { "" };
    let mark_src = if preview {
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(ICON)
        )
    } else {
        "favicon.png".into()
    };
    Ok(format!(
        r##"<!doctype html><html lang="{}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="{policy}"><meta name="color-scheme" content="{}"><title>{}</title>{assets}</head><body class="zim-archive{preview_class}" data-zim-template="{}"><a class="archive-skip" href="#collection">Skip to collection</a><div class="archive-shell"><header class="archive-topbar"><a class="archive-wordmark" href="{}"><span class="archive-mark" aria-hidden="true"><img src="{mark_src}" width="27" height="29" alt=""></span>ZIMSMITH<span class="wordmark-divider"></span>YOUR OFFLINE LIBRARY</a><span class="offline-badge"><span aria-hidden="true"></span>Available offline</span></header><main><section class="archive-hero" aria-labelledby="archive-title"><div class="hero-copy"><p class="archive-eyebrow">{name}</p><h1 id="archive-title">{}</h1><p class="archive-description">{}</p><div class="collection-meta"><span>{} items</span><span aria-hidden="true">/</span><span>Curated by {}</span>{sample}</div></div><div class="hero-art" aria-hidden="true"><div class="art-orbit"></div><div class="art-sheet sheet-one"><span>COLLECT</span><i></i><i></i><i></i></div><div class="art-sheet sheet-two"><span>DISCOVER</span><b>↗</b><i></i><i></i></div><span class="art-seal">YOURS.<br>ANYWHERE.</span></div></section>{capture_notice}<section class="archive-collection" id="collection" aria-labelledby="collection-title"><div class="collection-heading"><div><p class="archive-eyebrow">THE COLLECTION</p><h2 id="collection-title">{collection_heading}</h2></div><p class="archive-count" id="count" role="status">{} items to explore</p></div><form class="archive-search" id="search-form" role="search"><label for="search">Search this collection</label><div class="search-control"><span aria-hidden="true">⌕</span><input id="search" type="search" placeholder="Search titles and text…" autocomplete="off" spellcheck="false"><button id="clear-search" type="button" hidden>Clear</button><span class="search-hint" aria-hidden="true">SEARCH</span></div></form><ol class="archive-contents" id="contents">{cards}</ol><div class="archive-empty" id="empty-state" hidden><span aria-hidden="true">⌕</span><h3>No matches this time.</h3><p>Try another word or clear your search to see the whole collection.</p></div><noscript><p class="archive-noscript">Search needs JavaScript. All collection links work without it.</p></noscript></section></main><footer class="archive-footer"><div><span class="footer-mark" aria-hidden="true"><img src="{mark_src}" width="27" height="29" alt=""></span><p>A piece of the web.<br><strong>Always within reach.</strong></p></div><a class="source-link" href="{}" rel="noopener noreferrer">Visit original source <span aria-hidden="true">↗</span></a><p class="archive-credit">Made with Zimsmith<br>Open format. Endless possibilities.</p></footer></div>{scripts}</body></html>"##,
        escape(&config.language),
        if matches!(
            template,
            ArchiveTemplate::Midnight | ArchiveTemplate::ScreeningRoom
        ) {
            "dark"
        } else {
            "light"
        },
        escape(&config.title),
        template.id(),
        if preview { "#collection" } else { "index.html" },
        escape(&config.title),
        escape(&config.description),
        catalog.documents.len(),
        escape(&config.creator),
        catalog.documents.len(),
        escape(source)
    ))
}

pub fn render_nav(template: ArchiveTemplate, source: &str) -> String {
    format!("<nav class='zim-archive-nav' data-zim-template='{}' aria-label='Archive navigation'><a href='index.html'>← Collection contents</a><span>Saved for offline reading</span><a href='{}' rel='noopener noreferrer'>Original source ↗</a></nav>", template.id(), escape(source))
}

pub fn render_media(template: ArchiveTemplate, config: &NewJob, entry: &Entry) -> String {
    let file = entry.media_file.as_deref().unwrap_or("");
    let audio_only = [".mp3", ".m4a", ".ogg", ".wav"]
        .iter()
        .any(|ext| file.ends_with(ext));
    let poster = entry
        .thumbnail_file
        .as_ref()
        .map(|file| format!(" poster=\"{}\"", escape(file)))
        .unwrap_or_default();
    let player = if audio_only {
        format!(
            "<audio class='standalone-audio' controls preload='metadata' src='{}'></audio>",
            escape(file)
        )
    } else {
        format!(
            "<video id='video-track' controls playsinline preload='metadata'{poster} src='{}'></video>",
            escape(file)
        )
    };

    format!(
        r#"<!doctype html><html lang="{}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'self'; script-src 'self'; font-src 'self'; img-src 'self'; media-src 'self'; form-action 'none'; base-uri 'none'"><title>{}</title><link rel="stylesheet" href="studio.css"></head><body class="zim-archive" data-zim-template="{}">{}<main class="archive-media"><p class="archive-eyebrow">FROM YOUR COLLECTION</p><h1>{}</h1><p class="media-collection">{}</p><div class="media-player">{player}</div><section class="media-description"><h2>About this item</h2><p>{}</p></section><a class="source-link" href="{}" rel="noopener noreferrer">Visit original source ↗</a></main></body></html>"#,
        escape(&config.language),
        escape(&entry.title),
        template.id(),
        render_nav(template, &entry.url),
        escape(&entry.title),
        escape(&config.title),
        escape(&entry.text),
        escape(&entry.url)
    )
}

pub fn preview(template: ArchiveTemplate) -> anyhow::Result<String> {
    let mut config = NewJob { title: "A world worth exploring.".into(), description: "Field guides, small discoveries, and a few good ideas. Collected to take with you, wherever curiosity leads.".into(), creator: "The curious reader".into(), url: "https://example.com/".into(), kind: SourceKind::Website, max_items: 100, max_depth: 2, delay_ms: 2000, max_resource_mb: 256, language: "eng".into(), include_assets: true };
    if template == ArchiveTemplate::ScreeningRoom {
        config.kind = SourceKind::Media;
        config.title = "Ideas worth a closer look.".into();
        config.description =
            "Fresh perspectives, remarkable places, and stories to take with you.".into();
        config.creator = "The curious viewer".into();
    }
    let mut catalog = SearchCatalog::default();
    for (i, (title, text)) in [
        ("The art of noticing", "Slow down, look closer, and find the extraordinary in the everyday. A field guide to paying attention."),
        ("A walk through the wild", "Follow the quieter trails. Discover the plants, patterns, and stories hiding just beyond the path."),
        ("Small ideas, lasting change", "Good questions lead to unexpected places. A collection of practical ideas for a more thoughtful world."),
        ("Notes from elsewhere", "Stories from people and places that remind us how much there is still to discover.")
    ].into_iter().enumerate() {
        let entry = Entry { id: format!("sample-{i}"), url: config.url.clone(), title: title.into(), text: text.into(), depth: 0, selected: true, state: "done".into(), size: None, duration: None, error: None, assets: vec![], media_file: None, media_audio: None, thumbnail_url: None, thumbnail_file: None };
        let mut entry = entry;
        if template == ArchiveTemplate::ScreeningRoom {
            entry.duration = Some([752., 1184., 401., 946.][i]);
            entry.media_file = Some(format!("sample-{i}.mp4"));
        }
        catalog.add(&entry, &config.kind);
    }
    render_index(template, &config, &catalog, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_previews_are_self_contained_and_legacy_settings_keep_default_style() {
        for option in catalog() {
            let html = preview(option.id).unwrap();
            assert!(html.contains(&format!("data-zim-template=\"{}\"", option.id.id())));
            assert!(html.contains("data:font/woff2;base64,"));
            assert!(!html.contains("<script"));
            assert!(!html.contains("https://"));
            assert!(!html.contains("onclick="));
        }
        let settings: crate::Settings =
            serde_json::from_str(r#"{"auto_start_small_jobs":false,"auto_start_item_limit":7}"#)
                .unwrap();
        assert_eq!(settings.archive_template, ArchiveTemplate::Atlas);
        assert!(serde_json::from_str::<ArchiveTemplate>("\"unknown\"").is_err());
    }
    #[test]
    fn metadata_cannot_break_out_of_html_or_the_search_catalog() {
        let mut config = NewJob {
            title: "</script><script>alert('title')</script>".into(),
            description: "<img src=x onerror=alert(1)>".into(),
            creator: "A & B".into(),
            url: "https://example.com/".into(),
            kind: SourceKind::Website,
            max_items: 100,
            max_depth: 2,
            delay_ms: 2000,
            max_resource_mb: 256,
            language: "eng".into(),
            include_assets: true,
        };
        let catalog = SearchCatalog {
            documents: vec![Document {
                title: config.title.clone(),
                excerpt: config.description.clone(),
                url: "safe.html".into(),
                kind: "Article",
                detail: "1 min read".into(),
                thumbnail: None,
                media: None,
            }],
            ..Default::default()
        };
        for option in self::catalog() {
            let html = render_index(option.id, &config, &catalog, false).unwrap();
            assert!(!html.contains("<script>alert"));
            assert!(!html.contains("<img src=x"));
            assert!(html.contains("&lt;script&gt;"));
            assert!(html.contains("\\u003c/script\\u003e"));
            assert!(html.contains("A &amp; B"));
            assert!(!html.contains("data:font"));
        }
        config.title = "Audio collection".into();
        let mut entry = Entry {
            id: "audio".into(),
            title: "A short recording".into(),
            url: config.url.clone(),
            text: "Notes".into(),
            depth: 0,
            selected: true,
            state: "done".into(),
            size: None,
            duration: None,
            error: None,
            assets: vec![],
            media_file: Some("recording.mp3".into()),
            media_audio: None,
            thumbnail_url: None,
            thumbnail_file: None,
        };
        assert!(render_media(ArchiveTemplate::Midnight, &config, &entry)
            .contains("<audio class='standalone-audio'"));
        entry.media_file = Some("video.mp4".into());
        assert!(render_media(ArchiveTemplate::FieldNotes, &config, &entry)
            .contains("<video id='video-track'"));
    }
}
