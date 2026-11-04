use crate::{archive::escape, templates::SearchCatalog, web, Entry, Job, Record, SourceKind};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Summary {
    #[serde(default)]
    pub javascript_pages: usize,
    pub captured_items: usize,
    pub saved_assets: usize,
    pub omitted_assets: usize,
    pub excluded_assets: usize,
    pub failed_assets: usize,
    pub online_links: usize,
    pub subtitle_tracks: usize,
    pub caption_gaps: usize,
    pub search_limited: bool,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Validation {
    pub checksum_verified: bool,
    pub structure_verified: bool,
    pub checked_at: u64,
    pub archive_entries: u32,
}
#[derive(Serialize)]
struct Asset {
    url: String,
    state: String,
}
#[derive(Serialize)]
struct Item {
    title: String,
    url: String,
    state: String,
}
#[derive(Serialize)]
struct Report {
    summary: Summary,
    items: Vec<Item>,
    assets: Vec<Asset>,
    online_links: Vec<String>,
}
fn label(value: &str) -> String {
    web::http_url(value)
        .map(|u| format!("{}{}", u.origin().ascii_serialization(), u.path()))
        .unwrap_or_else(|_| "Unavailable URL".into())
}
pub async fn write(
    record: &Record,
    job: &Job,
    selected: &[&Entry],
    catalog: &SearchCatalog,
    paths: &BTreeMap<String, String>,
    content: &std::path::Path,
) -> Result<Summary> {
    let mut needed = BTreeSet::new();
    let mut todo: Vec<_> = selected.iter().flat_map(|e| e.assets.clone()).collect();
    if job.config.include_assets {
        while let Some(url) = todo.pop() {
            if !needed.insert(url.clone()) {
                continue;
            }
            if let Some(r) = job.resources.get(&url).filter(|r| r.state == "done") {
                todo.extend(r.dependencies.clone());
            }
        }
    }
    let assets: Vec<_> = needed
        .iter()
        .map(|url| Asset {
            url: label(url),
            state: job
                .resources
                .get(url)
                .map(|r| {
                    if r.state == "skipped" && r.error.is_some() {
                        "skipped_after_failure".into()
                    } else {
                        r.state.clone()
                    }
                })
                .unwrap_or_else(|| "missing".into()),
        })
        .collect();
    let mut summary = Summary {
        captured_items: selected.len(),
        saved_assets: assets.iter().filter(|r| r.state == "done").count(),
        omitted_assets: assets.iter().filter(|r| r.state != "done").count(),
        excluded_assets: assets.iter().filter(|r| r.state == "excluded").count(),
        failed_assets: assets
            .iter()
            .filter(|r| r.state == "failed" || r.state == "skipped_after_failure")
            .count(),
        search_limited: catalog.search_limited,
        ..Default::default()
    };
    let mut online = BTreeSet::new();
    let mut embedded_videos = 0;
    let mut limited = false;
    let selector = scraper::Selector::parse("a[href]").unwrap();
    for entry in selected {
        if let Some(rendered) = &entry.rendered {
            summary.javascript_pages += 1;
            for warning in &rendered.warnings {
                if summary.warnings.len() < 30 {
                    summary.warnings.push(format!("{}: {warning}", entry.title));
                }
            }
        }
        if job.config.capture.subtitles {
            summary.subtitle_tracks += entry.subtitles.len();
            summary.caption_gaps += usize::from(entry.subtitle_error.is_some());
        }
        let suspicious = entry.title.to_ascii_lowercase();
        if summary.warnings.len() < 30
            && (suspicious.contains("just a moment")
                || suspicious.contains("access denied")
                || suspicious.contains("captcha")
                || (entry.media_file.is_none()
                    && entry.text.split_whitespace().count() < 5
                    && entry.assets.is_empty()))
        {
            summary.warnings.push(format!(
                "Check this possibly empty or blocked capture: {}",
                entry.title
            ));
        }
        if job.config.kind == SourceKind::Website && entry.media_file.is_none() {
            let path = record.dir.join("cache").join(format!("{}.html", entry.id));
            let html = tokio::fs::read_to_string(path).await?;
            let base = web::http_url(
                &tokio::fs::read_to_string(
                    record.dir.join("cache").join(format!("{}.url", entry.id)),
                )
                .await
                .unwrap_or_else(|_| entry.url.clone()),
            )?;
            let document = scraper::Html::parse_document(&html);
            if !crate::snapshot::missing_catalogs(&document).is_empty() {
                summary.warnings.push(format!("Client-rendered store catalog not captured: {}. Product listings require the live site.",entry.title));
            }
            let videos = crate::embedded::discover(&document);
            for id in videos.keys() {
                let url = format!("https://www.youtube.com/watch?v={id}");
                if !paths.contains_key(&url) {
                    embedded_videos += 1;
                    online.insert(label(&url));
                }
            }
            for anchor in document.select(&selector) {
                let Some(url) = anchor
                    .value()
                    .attr("href")
                    .filter(|s| !s.starts_with('#'))
                    .and_then(|s| web::resolve(&base, s))
                else {
                    continue;
                };
                if !paths.contains_key(url.as_str()) {
                    if online.len() >= 5000 {
                        limited = true;
                        break;
                    }
                    online.insert(label(url.as_str()));
                }
            }
        }
    }
    if summary.javascript_pages > 0 {
        summary.warnings.push("Browser rendering loaded scripts and page resources during the scan. Browser transfer counts are estimates and may omit cross-origin responses; checkout, forms, and source JavaScript are not retained for offline use.".into());
    }
    summary.online_links = online.len();
    if embedded_videos > 0 {
        summary.warnings.push(format!("{embedded_videos} embedded YouTube videos are labeled as online links. Their thumbnails are saved; the movies were not downloaded in this website capture."));
    }
    if limited {
        summary
            .warnings
            .push("Online-only link list was limited to 5000 URLs.".into());
    }
    if summary.search_limited {
        summary.warnings.push("Portable collection search reached its 16 MB text / one-million-posting budget. Each detail page retains its full transcript.".into());
    }
    if summary.caption_gaps > 0 {
        summary.warnings.push(format!(
            "Captions were unavailable for {} selected items. Video/audio downloads are preserved.",
            summary.caption_gaps
        ));
    }
    let report = Report {
        summary: summary.clone(),
        items: selected
            .iter()
            .map(|e| Item {
                title: e.title.clone(),
                url: label(&e.url),
                state: e.state.clone(),
            })
            .collect(),
        assets,
        online_links: online.into_iter().collect(),
    };
    let counts = [
        ("Captured items", summary.captured_items),
        ("Saved assets", summary.saved_assets),
        ("Omitted assets", summary.omitted_assets),
        ("Excluded by settings", summary.excluded_assets),
        ("Failed assets", summary.failed_assets),
        ("Online-only links", summary.online_links),
        ("Browser-rendered pages", summary.javascript_pages),
        ("Subtitle tracks", summary.subtitle_tracks),
        ("Caption gaps", summary.caption_gaps),
    ]
    .iter()
    .map(|(name, n)| format!("<tr><th scope='row'>{name}</th><td>{n}</td></tr>"))
    .collect::<String>();
    let warnings = summary
        .warnings
        .iter()
        .map(|w| format!("<li>{}</li>", escape(w)))
        .collect::<String>();
    let assets = report
        .assets
        .iter()
        .map(|r| {
            format!(
                "<tr><td>{}</td><td>{}</td></tr>",
                escape(&r.url),
                escape(&r.state)
            )
        })
        .collect::<String>();
    let links = report
        .online_links
        .iter()
        .map(|url| format!("<li>{}</li>", escape(url)))
        .collect::<String>();
    let omission = if summary.omitted_assets > 0 {
        format!("<p><strong>Built with available files.</strong> {} assets were omitted. Some images, styles, or fonts may be missing. All selected pages are included.</p>",summary.omitted_assets)
    } else {
        String::new()
    };
    let html=format!("<!doctype html><html lang='en'><head><meta charset='utf-8'><meta name='viewport' content='width=device-width,initial-scale=1'><meta http-equiv='Content-Security-Policy' content=\"default-src 'none'; style-src 'self'; font-src 'self'; img-src 'self'\"><title>Capture report</title><link rel='stylesheet' href='studio.css'></head><body class='zim-archive'><main class='archive-media capture-report'><a href='index.html'>Back to collection</a><h1>Capture report</h1>{omission}<p>This report describes the saved content. Zimsmith checks the finished ZIM’s checksum and directory structure before marking it ready. Validation does not guarantee that every source page or media file displays correctly.</p><table><tbody>{counts}</tbody></table><ul>{warnings}</ul><h2>Assets</h2><table><thead><tr><th>URL</th><th>Result</th></tr></thead><tbody>{assets}</tbody></table><h2>Links still outside this archive</h2><p>These destinations were not captured. Query strings are omitted from this report.</p><ul>{links}</ul></main></body></html>");
    tokio::fs::write(content.join("capture-report.html"), html).await?;
    tokio::fs::write(
        content.join("capture-report.json"),
        serde_json::to_vec(&report)?,
    )
    .await?;
    Ok(summary)
}

pub async fn validate(record: &Record, path: &std::path::Path) -> Result<Validation> {
    use md5::{Digest, Md5};
    use tokio::io::AsyncReadExt;
    let archive = zimru::Archive::open(path).context("Could not reopen the finished ZIM")?;
    anyhow::ensure!(
        archive.check_dirent_ptrs()?
            && archive.check_dirent_order()?
            && archive.check_title_index()?
            && archive.check_cluster_ptrs()?
            && archive.check_mimetypes()?,
        "ZIM directory or cluster structure is invalid"
    );
    anyhow::ensure!(
        archive.main_path()? == "index.html",
        "ZIM is missing its contents page"
    );
    let index = archive.get_text("index.html")?;
    anyhow::ensure!(
        index.contains("catalog") && index.contains("search.js"),
        "ZIM is missing its search catalog"
    );
    let expected = archive.checksum()?;
    let count = archive.all_entry_count();
    drop(archive);
    let size = tokio::fs::metadata(path).await?.len();
    anyhow::ensure!(size >= 96, "ZIM is too short");
    let end = size - 16;
    let mut file = tokio::fs::File::open(path).await?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut at = 0;
    let mut digest = Md5::new();
    while at < end {
        let n = buffer.len().min((end - at) as usize);
        file.read_exact(&mut buffer[..n]).await?;
        digest.update(&buffer[..n]);
        at += n as u64;
        record
            .progress("Verifying finished ZIM checksum", at, Some(end), "bytes")
            .await;
        tokio::task::yield_now().await;
    }
    let actual: [u8; 16] = digest.finalize().into();
    anyhow::ensure!(
        actual == expected,
        "Finished ZIM checksum does not match; previous archive was preserved"
    );
    Ok(Validation {
        checksum_verified: true,
        structure_verified: true,
        checked_at: crate::now(),
        archive_entries: count,
    })
}
