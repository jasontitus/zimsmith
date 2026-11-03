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

pub async fn package(record: &Record) -> Result<()> {
    crate::thumbnails::prepare(record).await?;
    let job = record.job.lock().await.clone();
    record
        .progress("Preparing archive content", 0, None, "files")
        .await;
    let content = record.dir.join("content");
    if content.exists() {
        tokio::fs::remove_dir_all(&content).await?;
    }
    tokio::fs::create_dir_all(&content).await?;
    let selected: Vec<_> = job
        .entries
        .iter()
        .filter(|e| e.selected && e.state == "done")
        .collect();
    anyhow::ensure!(!selected.is_empty(), "No completed entries to package");
    let mut paths: BTreeMap<String, String> = selected
        .iter()
        .map(|e| (e.url.clone(), format!("{}.html", e.id)))
        .collect();
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
    if !omitted.is_empty() {
        let rows = omitted
            .iter()
            .map(|url| {
                let label = http_url(url)
                    .map(|u| format!("{}{}", u.origin().ascii_serialization(), u.path()))
                    .unwrap_or_else(|_| "Unavailable asset".into());
                format!("<li>{}</li>", escape(&label))
            })
            .collect::<String>();
        let report = format!("<!doctype html><html lang='en'><head><meta charset='utf-8'><meta name='viewport' content='width=device-width,initial-scale=1'><meta http-equiv='Content-Security-Policy' content=\"default-src 'none'; style-src 'self'; font-src 'self'; img-src 'self'\"><title>Capture report</title><link rel='stylesheet' href='studio.css'></head><body class='zim-archive'><main class='archive-media'><a href='index.html'>Back to collection</a><h1>Capture report</h1><p>Built with available files at your request. {} assets were omitted. Some images, styles, or fonts may be missing. All selected pages are included.</p><ul>{rows}</ul></main></body></html>", omitted.len());
        tokio::fs::write(content.join("capture-report.html"), report).await?;
    }
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
        let page = match job.config.kind {
            SourceKind::Website => {
                let html = tokio::fs::read_to_string(
                    record.dir.join("cache").join(format!("{}.html", entry.id)),
                )
                .await?;
                let effective = tokio::fs::read_to_string(
                    record.dir.join("cache").join(format!("{}.url", entry.id)),
                )
                .await
                .unwrap_or_else(|_| entry.url.clone());
                rewrite_page(&html, &http_url(&effective)?, &paths, job.archive_template)?
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
        catalog.add(&catalog_entry, &job.config.kind);
    }
    let index = templates::render_index(job.archive_template, &job.config, &catalog, false)?;
    tokio::fs::write(content.join("index.html"), index).await?;
    tokio::fs::write(content.join("search.js"), templates::SEARCH_JS).await?;
    if job.archive_template == ArchiveTemplate::ScreeningRoom {
        tokio::fs::write(content.join("gallery.js"), templates::GALLERY_JS).await?;
    }
    tokio::fs::write(content.join("studio.css"), templates::CSS).await?;
    tokio::fs::write(content.join("archive-sans.woff2"), templates::SANS).await?;
    tokio::fs::write(content.join("archive-display.woff2"), templates::DISPLAY).await?;
    tokio::fs::write(content.join("font-licenses.txt"), templates::FONT_LICENSE).await?;
    write_icon(&content.join("favicon.png"))?;
    record.log("info", "Offline pages, table of contents, and portable inverted search index ready. Writing ZIM locally with zimru.").await?;
    let partial = record.dir.join("archive.partial.zim");
    if partial.exists() {
        tokio::fs::remove_file(&partial).await?;
    }
    let mut creator = zimru::writer::Creator::new();
    creator
        .set_compression_level(3)
        .set_cluster_size_target(1024 * 1024)
        .set_max_in_flight_bytes(8 * 1024 * 1024)
        .set_streaming_encode_threshold(256 * 1024);
    creator.try_set_main_path("index.html")?;
    for (name, value) in [
        ("Title", job.config.title.as_str()),
        ("Description", &job.config.description),
        ("Language", &job.config.language),
        ("Creator", &job.config.creator),
        ("Publisher", "Zimsmith"),
        ("Source", &job.config.url),
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
    let final_path = record.dir.join("archive.zim");
    // Same-directory rename replaces atomically, preserving the old ZIM on failure.
    tokio::fs::rename(&partial, &final_path).await?;
    record.change(|j| {
        j.status = "completed".into(); j.output = Some(final_path.display().to_string()); j.output_bytes = Some(size);
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
    let parsed = scraper::Html::parse_document(html);
    let base_selector = scraper::Selector::parse("base[href]").unwrap();
    let base = parsed
        .select(&base_selector)
        .next()
        .and_then(|e| resolve(source, e.value().attr("href")?))
        .unwrap_or_else(|| source.clone());
    let style_paths = paths.clone();
    let style_base = base.clone();
    let page = rewrite_str(html, RewriteStrSettings { element_content_handlers: vec![
        element!("script, iframe, object, embed, base, meta[http-equiv], link[rel='preload'], link[rel='modulepreload']", |el| { el.remove(); Ok(()) }),
        element!("form", |el| { el.remove_and_keep_content(); Ok(()) }),
        element!("*", |el| {
            if ["img", "source"].contains(&el.tag_name().as_str()) && el.get_attribute("src").is_none() {
                let fallback = el.get_attribute("data-src").or_else(|| el.get_attribute("srcset").and_then(|s| s.split(',').next().and_then(|s| s.split_whitespace().next()).map(str::to_string)));
                if let Some(src) = fallback { el.set_attribute("src", &src)?; }
            }
            let attrs: Vec<_> = el.attributes().iter().map(|a| a.name()).collect();
            for attr in attrs { if attr.starts_with("on") || ["srcset", "integrity", "crossorigin", "nonce", "action", "formaction", "ping"].contains(&attr.as_str()) { el.remove_attribute(&attr); } }
            for attr in ["href", "src", "poster"] {
                let Some(value) = el.get_attribute(attr) else { continue; };
                if value.starts_with('#') || (value.starts_with("data:image/") && attr != "href") { continue; }
                let resolved = resolve(&base, &value);
                if let Some(path) = resolved.as_ref().and_then(|u| paths.get(u.as_str())) {
                    let fragment = base.join(&value).ok().and_then(|u| u.fragment().map(str::to_string)).map(|s| format!("#{s}")).unwrap_or_default();
                    el.set_attribute(attr, &format!("{path}{fragment}"))?;
                } else if attr == "href" && el.tag_name() == "a" {
                    if let Some(url) = resolved { el.set_attribute(attr, url.as_str())?; el.set_attribute("rel", "noopener noreferrer")?; }
                    else { el.remove_attribute(attr); }
                } else { el.remove_attribute(attr); }
            }
            if let Some(style) = el.get_attribute("style") { el.set_attribute("style", &rewrite_css(&style, &style_base, &style_paths))?; }
            Ok(())
        }),
        element!("meta[charset]", |el| { el.set_attribute("charset", "utf-8")?; Ok(()) }),
        element!("head", |el| { el.prepend("<link rel=\"stylesheet\" href=\"studio.css\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'self' data:; script-src 'none'; style-src 'self' 'unsafe-inline'; form-action 'none'; connect-src 'none'\">", lol_html::html_content::ContentType::Html); Ok(()) }),
        element!("body", |el| { el.prepend(&templates::render_nav(template, source.as_str()), lol_html::html_content::ContentType::Html); Ok(()) }),
    ], ..Default::default() })?;
    // Rewrite complete style blocks after parsing, so split streaming text chunks cannot break URLs.
    let re = regex::Regex::new(r"(?is)(<style[^>]*>)(.*?)(</style>)")?;
    Ok(re
        .replace_all(&page, |c: &regex::Captures| {
            format!("{}{}{}", &c[1], rewrite_css(&c[2], &base, paths), &c[3])
        })
        .into_owned())
}

fn write_icon(path: &std::path::Path) -> Result<()> {
    std::fs::write(path, templates::ICON)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
