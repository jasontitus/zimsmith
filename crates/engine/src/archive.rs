use crate::{
    model::*,
    templates,
    web::{http_url, resolve, rewrite_css},
    Record,
};
use anyhow::{Context, Result};
use lol_html::{element, rewrite_str, RewriteStrSettings};
use std::collections::{BTreeMap, BTreeSet};
use tokio::io::AsyncReadExt;

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub(crate) fn main_path(job: &Job) -> String {
    if job.archive_template == ArchiveTemplate::Website && job.config.kind == SourceKind::Website {
        if let Some(root) = job.entries.iter().find(|e| {
            e.url == job.config.url && e.selected && e.state == "done" && e.media_file.is_none()
        }) {
            return format!("{}.html", root.id);
        }
    }
    "index.html".into()
}

pub async fn package(record: &Record) -> Result<()> {
    crate::web::repair_saved_stylesheets(record).await?;
    crate::thumbnails::prepare(record).await?;
    let job = record.job.lock().await.clone();
    anyhow::ensure!(
        (1..=22).contains(&job.zstd_level),
        "Zstandard compression level must be 1–22"
    );
    record
        .progress("Preparing archive content", 0, None, "files")
        .await;
    let content = record.dir.join("content");
    if content.exists() {
        tokio::fs::remove_dir_all(&content).await?;
    }
    tokio::fs::create_dir_all(&content).await?;
    let mut selected: Vec<_> = job
        .entries
        .iter()
        .filter(|e| crate::embedded::approved(e, &job) && e.state == "done")
        .collect();
    selected.sort_by_key(|e| e.media_file.is_none());
    anyhow::ensure!(!selected.is_empty(), "No completed entries to package");
    let mut paths: BTreeMap<String, String> = selected
        .iter()
        .map(|e| (e.url.clone(), format!("{}.html", e.id)))
        .collect();
    // Generated inline players use these known local files, without resolving
    // them against the original website's URL.
    for entry in &selected {
        if let Some(file) = &entry.media_file {
            let file = if entry.media_audio.is_some() {
                format!("{}-combined.mp4", entry.id)
            } else {
                file.clone()
            };
            paths.insert(format!("urn:zimsmith:{file}"), file);
        }
        for file in entry.thumbnail_file.iter().chain(
            entry
                .subtitles
                .iter()
                .filter(|_| job.config.capture.subtitles)
                .map(|t| &t.file),
        ) {
            paths.insert(format!("urn:zimsmith:{file}"), file.clone());
        }
    }
    let mut needed = BTreeSet::new();
    let mut todo: Vec<_> = selected.iter().flat_map(|e| e.assets.clone()).collect();
    if job.config.include_assets {
        while let Some(url) = todo.pop() {
            if !needed.insert(url.clone()) {
                continue;
            }
            if let Some(resource) = job.resources.get(&url).filter(|r| r.state == "done") {
                paths.insert(url, resource.path.clone());
                todo.extend(resource.dependencies.clone());
            }
        }
    }
    let omitted: Vec<_> = needed
        .iter()
        .filter(|url| !job.resources.get(*url).is_some_and(|r| r.state == "done"))
        .collect();
    let omitted_count = omitted.len();
    for url in needed {
        let Some(resource) = job.resources.get(&url).filter(|r| r.state == "done") else {
            continue;
        };
        let from = record.dir.join("cache").join(&resource.path);
        if resource.mime.contains("text/css") {
            let css = tokio::fs::read_to_string(&from).await?;
            let base = tokio::fs::read_to_string(
                record
                    .dir
                    .join("cache")
                    .join(format!("{}.url", resource.path)),
            )
            .await
            .unwrap_or_else(|_| url.clone());
            tokio::fs::write(
                content.join(&resource.path),
                rewrite_css(&css, &http_url(&base)?, &paths),
            )
            .await?;
        } else {
            tokio::fs::copy(from, content.join(&resource.path)).await?;
        }
    }
    let mut catalog = templates::SearchCatalog::default();
    catalog.omitted_assets = omitted_count;
    catalog.capture_report = true;
    let embedded_players = crate::embedded::bindings(&job);
    for (index, entry) in selected.iter().enumerate() {
        record
            .progress(
                format!("Preparing and indexing {}", entry.title),
                index as u64,
                Some(selected.len() as u64),
                "items",
            )
            .await;
        let mut catalog_entry = (*entry).clone();
        if !job.config.capture.subtitles {
            catalog_entry.subtitles.clear();
            catalog_entry.transcript.clear();
            catalog_entry.transcript_file = None;
        }
        if let Some(file) = &catalog_entry.transcript_file {
            let path = record.dir.join("media").join(file);
            anyhow::ensure!(
                tokio::fs::metadata(&path).await?.len() <= 4 * 1024 * 1024,
                "Transcript exceeds safe parsing size"
            );
            catalog_entry.transcript = serde_json::from_slice(&tokio::fs::read(path).await?)?;
        }
        for track in &catalog_entry.subtitles {
            tokio::fs::copy(
                record.dir.join("media").join(&track.file),
                content.join(&track.file),
            )
            .await?;
        }
        let item_kind = if entry.media_file.is_some() {
            SourceKind::Media
        } else {
            job.config.kind.clone()
        };
        let page = match item_kind {
            SourceKind::Website => {
                let html = tokio::fs::read_to_string(
                    record.dir.join("cache").join(format!("{}.html", entry.id)),
                )
                .await?;
                catalog_entry.text =
                    crate::web::visible_text(&scraper::Html::parse_document(&html));
                let effective = tokio::fs::read_to_string(
                    record.dir.join("cache").join(format!("{}.url", entry.id)),
                )
                .await
                .unwrap_or_else(|_| entry.url.clone());
                rewrite_page(
                    &crate::embedded::rewrite(&html, &embedded_players)?,
                    &http_url(&effective)?,
                    &paths,
                    job.archive_template,
                )?
            }
            SourceKind::Media => {
                let file = entry
                    .media_file
                    .as_ref()
                    .context("Media file checkpoint missing")?;
                if let Some(audio) = &entry.media_audio {
                    let combined = format!("{}-combined.mp4", entry.id);
                    record
                        .progress(
                            format!("Combining audio and video: {}", entry.title),
                            index as u64,
                            Some(selected.len() as u64),
                            "items",
                        )
                        .await;
                    crate::mux::combine(
                        record,
                        record.dir.join("media").join(file),
                        record.dir.join("media").join(audio),
                        content.join(&combined),
                    )
                    .await?;
                    catalog_entry.media_file = Some(combined);
                    catalog_entry.media_audio = None;
                } else {
                    tokio::fs::copy(record.dir.join("media").join(file), content.join(file))
                        .await?;
                }
                if let Some(poster) = &entry.thumbnail_file {
                    tokio::fs::copy(record.dir.join("media").join(poster), content.join(poster))
                        .await?;
                }
                templates::render_media(job.archive_template, &job.config, &catalog_entry)
            }
        };
        tokio::fs::write(content.join(format!("{}.html", entry.id)), page).await?;
        catalog.add(&catalog_entry, &item_kind);
    }
    let report =
        crate::capture_report::write(record, &job, &selected, &catalog, &paths, &content).await?;
    let index = templates::render_index(job.archive_template, &job.config, &catalog, false)?;
    tokio::fs::write(content.join("index.html"), index).await?;
    tokio::fs::write(content.join("search.js"), templates::SEARCH_JS).await?;
    tokio::fs::write(content.join("media.js"), templates::MEDIA_JS).await?;
    if job.archive_template == ArchiveTemplate::ScreeningRoom {
        tokio::fs::write(content.join("gallery.js"), templates::GALLERY_JS).await?;
    }
    tokio::fs::write(content.join("studio.css"), templates::CSS).await?;
    tokio::fs::write(content.join("archive-sans.woff2"), templates::SANS).await?;
    tokio::fs::write(content.join("archive-display.woff2"), templates::DISPLAY).await?;
    tokio::fs::write(content.join("font-licenses.txt"), templates::FONT_LICENSE).await?;
    write_icon(&content.join("favicon.png"))?;
    record.log("info", format!("Offline pages, table of contents, and portable inverted search index ready. Writing ZIM locally with zimru, Zstandard level {}.", job.zstd_level)).await?;
    let partial = record.dir.join("archive.partial.zim");
    if partial.exists() {
        tokio::fs::remove_file(&partial).await?;
    }
    let mut creator = zimru::writer::Creator::new();
    creator
        .set_compression_level(job.zstd_level)
        .set_cluster_size_target(1024 * 1024)
        .set_max_in_flight_bytes(8 * 1024 * 1024)
        .set_streaming_encode_threshold(256 * 1024);
    creator.try_set_main_path(main_path(&job))?;
    for (name, value) in [
        ("Title", job.config.title.as_str()),
        ("Description", &job.config.description),
        ("Language", &job.config.language),
        ("Creator", &job.config.creator),
        ("Publisher", "Zimsmith"),
        (
            "Source",
            if job.import.is_some() {
                "Local saved content"
            } else {
                &job.config.url
            },
        ),
        ("Scraper", "Zimsmith 0.1"),
    ] {
        creator.try_add_metadata(name, value)?;
    }
    let date = httpdate::fmt_http_date(std::time::SystemTime::now());
    // Metadata's date is emitted in ISO format without adding a clock dependency.
    let parts: Vec<_> = date.split_whitespace().collect();
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| *m == parts[2])
    .unwrap_or(0)
        + 1;
    creator.try_add_metadata("Date", format!("{}-{:02}-{}", parts[3], month, parts[1]))?;
    creator.try_add_metadata("Name", format!("zim-studio-{}", job.id))?;
    creator.try_add_illustration(48, tokio::fs::read(content.join("favicon.png")).await?)?;
    creator.start_writing(&partial)?;
    #[cfg(feature = "kiwix-index")]
    add_kiwix_indexes(record, &job, &selected, &content, &mut creator).await?;
    let mut directory = tokio::fs::read_dir(&content).await?;
    let mut files = Vec::new();
    while let Some(file) = directory.next_entry().await? {
        if file.file_type().await?.is_file() {
            files.push(file);
        }
    }
    let mut buffer = vec![0u8; 64 * 1024];
    for (index, file) in files.iter().enumerate() {
        let filename = file.file_name().to_string_lossy().into_owned();
        let meta = file.metadata().await?;
        if !meta.is_file() {
            continue;
        }
        record
            .progress(
                format!("Writing {filename} to the ZIM"),
                index as u64,
                Some(files.len() as u64),
                "files",
            )
            .await;
        let title = selected
            .iter()
            .find(|e| filename == format!("{}.html", e.id))
            .map(|e| e.title.as_str())
            .unwrap_or(if filename == "index.html" {
                &job.config.title
            } else {
                ""
            });
        let mime = job
            .resources
            .values()
            .find(|r| r.path == filename)
            .map(|r| {
                r.mime
                    .split(';')
                    .next()
                    .unwrap_or("application/octet-stream")
                    .to_string()
            })
            .unwrap_or_else(|| {
                mime_guess::from_path(&filename)
                    .first_or_octet_stream()
                    .to_string()
            });
        let compress = !mime.starts_with("video/")
            && !mime.starts_with("audio/")
            && !mime.starts_with("image/");
        creator.begin_chunked_item(
            None,
            filename,
            title.to_string(),
            mime,
            Some(meta.len()),
            Some(compress),
        )?;
        let mut input = tokio::fs::File::open(file.path()).await?;
        let mut written = 0u64;
        let mut last_update = std::time::Instant::now();
        loop {
            let read = input.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            creator.chunked_item_chunk(&buffer[..read])?;
            written += read as u64;
            if last_update.elapsed() >= std::time::Duration::from_millis(500) {
                record.transfer_progress(written, Some(meta.len())).await;
                last_update = std::time::Instant::now();
            }
            tokio::task::yield_now().await;
        }
        creator.end_chunked_item()?;
    }
    record
        .progress(
            "Finalizing the ZIM directory and checksum",
            files.len() as u64,
            None,
            "files",
        )
        .await;
    creator.finish_writing()?;
    let size = tokio::fs::metadata(&partial).await?.len();
    anyhow::ensure!(size > 80, "Writer produced an empty archive");
    let mut header = [0u8; 4];
    tokio::fs::File::open(&partial)
        .await?
        .read_exact(&mut header)
        .await?;
    anyhow::ensure!(
        header.starts_with(&[0x5a, 0x49, 0x4d, 0x04]),
        "Writer output is not a ZIM file"
    );
    let validation = crate::capture_report::validate(record, &partial).await?;
    let final_path = record.dir.join("archive.zim");
    // Same-directory rename replaces atomically, preserving the old ZIM on failure.
    tokio::fs::rename(&partial, &final_path).await?;
    record.change(|j| {
        j.status = "completed".into(); j.output = Some(final_path.display().to_string()); j.output_bytes = Some(size);
        j.capture_report=Some(report); j.validation=Some(validation);
        j.error = None; j.log("info", format!("Archive ready: {size} bytes. Includes {} entries, table of contents, and full-text index.", selected.len()));
        j.progress.label = "Archive complete".into(); j.progress.total = Some(files.len() as u64); j.progress.completed = files.len() as u64; j.progress.updated_at = now();
    }).await
}

#[cfg(feature = "kiwix-index")]
async fn add_kiwix_indexes(
    record: &Record,
    job: &Job,
    selected: &[&Entry],
    content: &std::path::Path,
    creator: &mut zimru::writer::Creator,
) -> Result<()> {
    use xapianbuilder::{
        parse::ParsedDoc, AccentRule, Builder, BuilderOptions, FulltextDocument, Mode,
    };
    for (mode, name) in [(Mode::Fulltext, "fulltext"), (Mode::Title, "title")] {
        let path = record.dir.join(format!("{name}.xapian"));
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }
        let builder = Builder::new(
            &path,
            BuilderOptions {
                language: &job.config.language,
                stopwords: "",
                stemmer: "",
                accent_rule: AccentRule::Libzim,
                keep_termlists: false,
                mode,
            },
        )?;
        for entry in selected {
            let page_path = format!("{}.html", entry.id);
            match mode {
                Mode::Title => builder.add_title(&page_path, &entry.title, "", "")?,
                Mode::Fulltext => {
                    let data = tokio::fs::read(content.join(&page_path)).await?;
                    let parsed = ParsedDoc::parse(&data)?;
                    if parsed.indexing_allowed() {
                        builder.add_fulltext(FulltextDocument {
                            path: &page_path,
                            title: &entry.title,
                            content: parsed.content().as_bytes(),
                            keywords: parsed.keywords().as_bytes(),
                            word_count: parsed.word_count(),
                            geo: parsed.geo(),
                            language: "",
                        })?;
                    }
                }
            }
            tokio::task::yield_now().await;
        }
        builder.finalize()?;
        let data = tokio::fs::read(&path).await?;
        creator.try_add_item(
            zimru::writer::Item::in_namespace(
                b'X',
                format!("{name}/xapian"),
                "",
                "application/octet-stream+xapian",
                data,
            )
            .with_compress(false),
        )?;
    }
    Ok(())
}

fn rewrite_page(
    html: &str,
    source: &url::Url,
    paths: &BTreeMap<String, String>,
    template: ArchiveTemplate,
) -> Result<String> {
    let html = crate::images::prepare(html)?;
    let html = html.as_str();
    let parsed = scraper::Html::parse_document(html);
    let base_selector = scraper::Selector::parse("base[href]").unwrap();
    let base = parsed
        .select(&base_selector)
        .next()
        .and_then(|e| resolve(source, e.value().attr("href")?))
        .unwrap_or_else(|| source.clone());
    let local_paths: BTreeSet<_> = paths.values().cloned().collect();
    let style_paths = paths.clone();
    let style_base = base.clone();
    let prepared = crate::snapshot::prepare(html, &parsed)?;
    // Allow only our exact preview helper; captured scripts remain disabled.
    let has_previews = prepared.contains("data-zim-poster=");
    let script_policy = if has_previews {
        templates::poster_policy()
    } else {
        "'none'".into()
    };
    let head = format!("<meta charset=\"utf-8\"><link rel=\"stylesheet\" href=\"studio.css\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self' data:; script-src {script_policy}; style-src 'self' 'unsafe-inline'; form-action 'none'; connect-src 'none'\">");
    let mut script_added = false;
    let mut page = rewrite_str(&prepared, RewriteStrSettings { element_content_handlers: vec![
        element!("script, iframe, object, embed, base, meta[http-equiv], link[rel='preload'], link[rel='modulepreload']", |el| { el.remove(); Ok(()) }),
        element!("form", |el| { el.remove_and_keep_content(); Ok(()) }),
        element!("*", |el| {
            let mut classes = el.get_attribute("class").unwrap_or_default();
            let role = el.get_attribute("data-animation-role");
            if crate::snapshot::reveal(&classes, role.as_deref()) {
                el.set_attribute("data-animation-state", "active")?;
                if role.as_deref() == Some("image") && !classes.split_whitespace().any(|c| c == "loaded") { classes.push_str(" loaded"); el.set_attribute("class", &classes)?; }
            }
            if el.get_attribute("data-aos").is_some() && !classes.split_whitespace().any(|c| c == "aos-animate") { classes.push_str(" aos-animate"); el.set_attribute("class", &classes)?; }
            if ["img", "source"].contains(&el.tag_name().as_str()) && el.get_attribute("src").is_none() {
                let fallback = el.get_attribute("data-src").or_else(|| el.get_attribute("srcset").and_then(|s| s.split(',').next().and_then(|s| s.split_whitespace().next()).map(str::to_string)));
                if let Some(src) = fallback { el.set_attribute("src", &src)?; }
            }
            if el.tag_name() == "img" && !el.get_attribute("src").and_then(|s| resolve(&base, &decode_attribute(&s))).is_some_and(|u| paths.contains_key(u.as_str())) {
                if let Some(original) = el.get_attribute("data-zim-original-src").filter(|s| resolve(&base, &decode_attribute(s)).is_some_and(|u| paths.contains_key(u.as_str()))) { el.set_attribute("src", &original)?; }
            }
            el.remove_attribute("data-zim-original-src");
            let attrs: Vec<_> = el.attributes().iter().map(|a| a.name()).collect();
            for attr in attrs { if attr.starts_with("on") || ["srcset", "integrity", "crossorigin", "nonce", "action", "formaction", "ping"].contains(&attr.as_str()) { el.remove_attribute(&attr); } }
            for attr in ["href", "src", "poster"] {
                let Some(value) = el.get_attribute(attr) else { continue; };
                let value = decode_attribute(&value);
                if value.starts_with('#') || (value.starts_with("data:image/") && attr != "href") { continue; }
                if local_paths.contains(&value) { continue; }
                let resolved = resolve(&base, &value);
                if let Some(path) = resolved.as_ref().and_then(|u| paths.get(u.as_str())) {
                    let fragment = base.join(&value).ok().and_then(|u| u.fragment().map(str::to_string)).map(|s| format!("#{s}")).unwrap_or_default();
                    el.set_attribute(attr, &format!("{path}{fragment}"))?;
                } else if attr == "href" && el.tag_name() == "a" {
                    if let Some(url) = resolved { el.set_attribute(attr, url.as_str())?; el.set_attribute("rel", "noopener noreferrer")?; }
                    else { el.remove_attribute(attr); }
                } else { el.remove_attribute(attr); }
            }
            if let Some(style) = el.get_attribute("style") { el.set_attribute("style", &rewrite_css(&decode_attribute(&style), &style_base, &style_paths))?; }
            Ok(())
        }),
        element!("meta[charset]", |el| { el.set_attribute("charset", "utf-8")?; Ok(()) }),
        element!("head", |el| { el.prepend(&head, lol_html::html_content::ContentType::Html); Ok(()) }),
        element!("body", |el| {
            el.prepend(&templates::render_nav(template, source.as_str()), lol_html::html_content::ContentType::Html);
            if template == ArchiveTemplate::Website { el.append("<footer class=\"zim-site-footer\"><a href=\"index.html\">Search &amp; collection contents</a></footer>",lol_html::html_content::ContentType::Html); }
            if has_previews { el.append(&templates::poster_script(), lol_html::html_content::ContentType::Html); script_added = true; }
            Ok(())
        }),
    ], ..Default::default() })?;
    if has_previews && !script_added {
        page.push_str(&templates::poster_script());
    }
    // Rewrite complete style blocks after parsing, so split streaming text chunks cannot break URLs.
    let re = regex::Regex::new(r"(?is)(<style[^>]*>)(.*?)(</style>)")?;
    Ok(re
        .replace_all(&page, |c: &regex::Captures| {
            format!("{}{}{}", &c[1], rewrite_css(&c[2], &base, paths), &c[3])
        })
        .into_owned())
}

// lol_html preserves source character references; the crawler's HTML5 parser
// decodes them. Resolve attributes with the same HTML5 attribute semantics.
fn decode_attribute(value: &str) -> String {
    if !value.contains('&') {
        return value.into();
    }
    let fragment = scraper::Html::parse_fragment(&format!(
        "<i data-value=\"{}\"></i>",
        value.replace('"', "&#34;")
    ));
    fragment
        .select(&scraper::Selector::parse("i[data-value]").unwrap())
        .next()
        .and_then(|e| e.value().attr("data-value"))
        .unwrap_or(value)
        .into()
}

fn write_icon(path: &std::path::Path) -> Result<()> {
    std::fs::write(path, templates::ICON)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_images_do_not_keep_script_dependent_loading_markers() {
        let paths = BTreeMap::from([("https://example.test/photo.jpg".into(), "photo.jpg".into())]);
        let html = "<html><body><img class='ecommerce-product-image ecommerce-product-image--loading' src='/photo.jpg'><img class='photo lazyloading' src='/photo.jpg'><div class='intentionally-hidden'>Other content</div></body></html>";
        let output = rewrite_page(
            html,
            &http_url("https://example.test/").unwrap(),
            &paths,
            ArchiveTemplate::Website,
        )
        .unwrap();
        assert!(!output.contains("ecommerce-product-image--loading"));
        assert!(!output.contains("lazyloading"));
        assert!(output.contains("photo lazyloaded"));
        assert!(output.contains("ecommerce-product-image"));
        assert!(output.contains("intentionally-hidden"));
        assert!(output.contains("src=\"photo.jpg\""));
    }
    #[test]
    fn restores_static_animations_menus_embeds_and_entity_escaped_asset_urls() {
        let source = http_url("https://example.test/").unwrap();
        let paths = BTreeMap::from([
            (
                "https://example.test/font.css?a=1&b=2".into(),
                "font.css".into(),
            ),
            ("https://example.test/about".into(), "about.html".into()),
        ]);
        let html = r#"<html><head><link rel="stylesheet" href="/font.css?a=1&amp;b=2"></head><body><button class="block-header__hamburger-menu">Menu</button><div class="block-header-layout-mobile__dropdown"><a href="/about">About</a></div><div class="transition transition--slide"><h1>Visible content</h1><div data-animation-role="image"></div></div><div class="video" id="movie"><img src="https://i.ytimg.com/vi/abcdefghijk/hqdefault.jpg"><button class="video__play--youtube"></button></div><script>throw 1</script></body></html>"#;
        let output = rewrite_page(html, &source, &paths, ArchiveTemplate::Atlas).unwrap();
        assert!(!crate::web::inspect(html, &source).text.contains("throw 1"));
        let dynamic = rewrite_page(
            "<html><body><div class='block-product-list' id='catalog'></div></body></html>",
            &source,
            &paths,
            ArchiveTemplate::Atlas,
        )
        .unwrap();
        assert!(dynamic.contains("Store catalog not captured"));
        let doc = scraper::Html::parse_document(&output);
        assert_eq!(
            doc.select(&scraper::Selector::parse("link[rel=stylesheet]").unwrap())
                .last()
                .unwrap()
                .value()
                .attr("href"),
            Some("font.css")
        );
        assert!(output.contains("data-animation-state=\"active\""));
        assert!(output.contains("class=\" loaded\"") || output.contains("class=\"loaded\""));
        assert!(output.contains("class=\"zim-menu-toggle\""));
        assert!(output.contains("href=\"about.html\""));
        assert!(output.contains("https://youtu.be/abcdefghijk"));
        assert!(output.contains("Watch video online"));
        assert!(!output.contains("<script"));
        assert!(!output.contains("video__play--youtube"));
    }
    #[test]
    fn rewrites_offline_links_and_removes_active_content() {
        let paths = BTreeMap::from([
            ("https://example.com/next".into(), "next.html".into()),
            ("https://example.com/img.png".into(), "img.png".into()),
        ]);
        let out = rewrite_page("<html><head><title>A</title><script>alert(1)</script></head><body onload='evil()'><a href='/next#part'>Next</a><img src='/img.png' srcset='x 2x'><iframe src='https://evil.test'></iframe></body></html>", &http_url("https://example.com/").unwrap(), &paths, ArchiveTemplate::Atlas).unwrap();
        assert!(out.contains("next.html#part"));
        assert!(out.contains("src=\"img.png\""));
        assert!(!out.contains("onload"));
        assert!(!out.contains("<script"));
        assert!(!out.contains("<iframe"));
        assert!(!out.contains("srcset"));
    }
}
