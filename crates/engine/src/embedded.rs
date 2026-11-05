//! Discover approved external media without turning arbitrary links into downloads.
use crate::{archive::escape, native_media, web, Entry, Job};
use anyhow::Result;
use lol_html::{element, rewrite_str, RewriteStrSettings};
use scraper::{Html, Selector};
use std::collections::BTreeMap;

pub fn video_id(value: &str) -> Option<String> {
    let url = if value.starts_with("//") {
        web::http_url(&format!("https:{value}")).ok()?
    } else {
        web::http_url(value).ok()?
    };
    let host = url.host_str()?;
    if !native_media::youtube(&url)
        && !matches!(host, "youtube-nocookie.com" | "www.youtube-nocookie.com")
    {
        return None;
    }
    let id = if host == "youtu.be" {
        url.path().trim_start_matches('/').to_owned()
    } else if url.path() == "/watch" {
        url.query_pairs().find(|(k, _)| k == "v")?.1.into_owned()
    } else {
        url.path().strip_prefix("/embed/")?.to_owned()
    };
    (id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')))
    .then_some(id)
}

pub fn discover(document: &Html) -> BTreeMap<String, String> {
    let mut videos = BTreeMap::new();
    for frame in document.select(&Selector::parse("iframe, embed, object").unwrap()) {
        let Some(id) = ["src", "data-src", "data"]
            .into_iter()
            .find_map(|attr| frame.value().attr(attr).and_then(video_id))
        else {
            continue;
        };
        videos.insert(
            id,
            frame
                .value()
                .attr("title")
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("Embedded YouTube video")
                .chars()
                .take(300)
                .collect(),
        );
    }
    for url in crate::snapshot::video_links(document).values() {
        if let Some(id) = video_id(url) {
            videos
                .entry(id)
                .or_insert_with(|| "Embedded YouTube video".into());
        }
    }
    videos
}

pub fn register(job: &mut Job, html: &str, parent: &str, depth: usize) {
    if !job.config.capture.embedded_videos || !job.config.capture.media {
        return;
    }
    for (id, title) in discover(&Html::parse_document(html)) {
        let mut entry = native_media::entry(&id, title, None, String::new(), false);
        if web::http_url(&entry.url).is_ok_and(|u| job.config.capture.excluded(&u)) {
            continue;
        }
        if let Some(existing) = job.entries.iter_mut().find(|e| e.url == entry.url) {
            if !existing.embedded_in.iter().any(|p| p == parent) {
                existing.embedded_in.push(parent.into());
            }
            continue;
        }
        if job.entries.len() >= job.config.max_items {
            job.scan_truncated = true;
            continue;
        }
        entry.depth = depth;
        entry.embedded_in.push(parent.into());
        entry.selected = job.config.page_allowed(parent);
        job.entries.push(entry);
    }
}

pub fn approved(entry: &Entry, _job: &Job) -> bool {
    entry.selected
}

fn player(entry: &Entry, subtitles: bool) -> String {
    let Some(file) = &entry.media_file else {
        return String::new();
    };
    let file = if entry.media_audio.is_some() {
        format!("{}-combined.mp4", entry.id)
    } else {
        file.clone()
    };
    let poster = entry
        .thumbnail_file
        .as_ref()
        .map(|p| {
            format!(
                " poster=\"{}\" data-zim-poster=\"{}\"",
                escape(p),
                escape(p)
            )
        })
        .unwrap_or_default();
    let tracks = entry
        .subtitles
        .iter()
        .filter(|_| subtitles)
        .map(|t| {
            format!(
                "<track kind=\"subtitles\" src=\"{}\" srclang=\"{}\" label=\"{}\">",
                escape(&t.file),
                escape(&t.language),
                escape(&t.label)
            )
        })
        .collect::<String>();
    format!("<figure class=\"zim-embedded-video\"><video controls playsinline preload=\"metadata\" src=\"{}\"{poster}>{tracks}</video><figcaption><a href=\"{}.html\">{}</a></figcaption></figure>", escape(&file), escape(&entry.id), escape(&entry.title))
}

pub fn bindings(job: &Job) -> BTreeMap<String, String> {
    job.entries
        .iter()
        .filter(|e| e.selected && e.state == "done" && e.media_file.is_some())
        .filter_map(|e| video_id(&e.url).map(|id| (id, player(e, job.config.capture.subtitles))))
        .collect()
}

/// Runs before untrusted iframes/scripts are removed. Only local saved media is inserted.
pub fn rewrite(html: &str, saved: &BTreeMap<String, String>) -> Result<String> {
    let cards = crate::snapshot::video_links(&Html::parse_document(html));
    let output = rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![
                element!("iframe, embed, object", |el| {
                    if let Some(id) = ["src", "data-src", "data"]
                        .into_iter()
                        .find_map(|a| el.get_attribute(a).and_then(|v| video_id(&v)))
                    {
                        let replacement = saved.get(&id).cloned().unwrap_or_else(|| format!("<p class=\"zim-online-video\"><a href=\"https://www.youtube.com/watch?v={id}\">Watch video online ↗</a> · Video not saved</p>"));
                        el.replace(&replacement, lol_html::html_content::ContentType::Html);
                    }
                    Ok(())
                }),
                element!(".video[id]", |el| {
                    if let Some(player) = el
                        .get_attribute("id")
                        .and_then(|id| cards.get(&id))
                        .and_then(|url| video_id(url))
                        .and_then(|id| saved.get(&id))
                    {
                        el.set_inner_content(player, lol_html::html_content::ContentType::Html);
                    }
                    Ok(())
                }),
            ],
            ..Default::default()
        },
    )?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_and_deduplicates_known_embed_hosts() {
        let doc = Html::parse_document(
            r#"<iframe src="https://www.youtube-nocookie.com/embed/abcdefghijk?rel=0" title="Talk"></iframe><iframe data-src="//www.youtube.com/embed/abcdefghijk"></iframe><iframe src="https://youtube.com.evil.test/embed/abcdefghijk"></iframe>"#,
        );
        assert_eq!(discover(&doc).len(), 1);
        assert!(video_id("https://example.test/embed/abcdefghijk").is_none());
        assert!(video_id("/embed/abcdefghijk").is_none());
        assert!(video_id("https://youtube.com/embed/../secret").is_none());
        assert_eq!(
            video_id("https://youtu.be/abcdefghijk"),
            Some("abcdefghijk".into())
        );
    }
}
