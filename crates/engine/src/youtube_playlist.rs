//! Playlist metadata compatibility for both classic renderers and YouTube's
//! current direct lockup lists. Requests still use the on-device RustyPipe client.
use crate::{native_media::entry, web, Entry, Record};
use anyhow::{bail, Context, Result};
use rustypipe::client::{ClientType, RustyPipe};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Page {
    entries: Vec<Entry>,
    next: Option<String>,
    visitor: Option<String>,
    incomplete: bool,
}

fn text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            value
                .get("simpleText")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| {
            value
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| {
            value
                .get("runs")
                .and_then(Value::as_array)
                .map(|runs| {
                    runs.iter()
                        .filter_map(|run| run.get("text").and_then(Value::as_str))
                        .collect()
                })
                .unwrap_or_default()
        })
}

fn duration(value: &str) -> Option<f64> {
    let parts = value
        .split(':')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    if !(2..=3).contains(&parts.len()) || parts.iter().skip(1).any(|part| *part >= 60) {
        return None;
    }
    Some(
        parts
            .into_iter()
            .fold(0u64, |total, part| total * 60 + part) as f64,
    )
}

fn badge(value: &Value) -> Option<String> {
    match value {
        Value::Object(object) => {
            if let Some(badge) = object.get("thumbnailBadgeViewModel") {
                return badge.get("text").and_then(Value::as_str).map(str::to_owned);
            }
            object.values().find_map(badge)
        }
        Value::Array(values) => values.iter().find_map(badge),
        _ => None,
    }
}

fn collect(value: &Value, page: &mut Page) {
    if let Some(values) = value.as_array() {
        for value in values {
            collect(value, page);
        }
        return;
    }
    let Some(object) = value.as_object() else {
        return;
    };
    if let Some(item) = object.get("lockupViewModel") {
        if item["contentType"].as_str() != Some("LOCKUP_CONTENT_TYPE_VIDEO") {
            return;
        }
        let id = item["contentId"].as_str().unwrap_or_default();
        let title = text(&item["metadata"]["lockupMetadataViewModel"]["title"]);
        if id.is_empty() || title.is_empty() {
            page.incomplete = true;
            return;
        }
        let badge = badge(&item["contentImage"]);
        let unavailable = badge
            .as_deref()
            .is_some_and(|b| matches!(b, "LIVE" | "UPCOMING" | "PREMIERE"))
            || matches!(title.as_str(), "[Private video]" | "[Deleted video]");
        page.entries.push(entry(
            id,
            title,
            badge.as_deref().and_then(duration),
            String::new(),
            unavailable,
        ));
        return;
    }
    for kind in [
        "playlistVideoRenderer",
        "videoRenderer",
        "gridVideoRenderer",
    ] {
        if let Some(item) = object.get(kind) {
            let id = item["videoId"].as_str().unwrap_or_default();
            let title = text(&item["title"]);
            if id.is_empty() || title.is_empty() {
                page.incomplete = true;
                return;
            }
            let length = item["lengthSeconds"]
                .as_str()
                .and_then(|v| v.parse::<f64>().ok())
                .or_else(|| duration(&text(&item["lengthText"])));
            let unavailable = item["isPlayable"].as_bool() == Some(false)
                || item.get("upcomingEventData").is_some_and(|v| !v.is_null())
                || matches!(title.as_str(), "[Private video]" | "[Deleted video]");
            page.entries.push(entry(
                id,
                title,
                length,
                text(&item["descriptionSnippet"]),
                unavailable,
            ));
            return;
        }
    }
    if let Some(item) = object.get("continuationItemRenderer") {
        page.next = item
            .pointer("/continuationEndpoint/continuationCommand/token")
            .or_else(|| item.pointer("/button/buttonRenderer/command/continuationCommand/token"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if page.next.is_none() {
            page.incomplete = true;
        }
        return;
    }
    // Walk only list containers, never sidebar recommendations or nested links.
    for key in [
        "contents",
        "items",
        "content",
        "sectionListRenderer",
        "itemSectionRenderer",
        "playlistVideoListRenderer",
        "richGridRenderer",
        "richItemRenderer",
        "appendContinuationItemsAction",
        "reloadContinuationItemsCommand",
        "continuationItems",
    ] {
        if let Some(child) = object.get(key) {
            collect(child, page);
        }
    }
}

fn parse(response: &str, continuation: bool) -> Result<Page> {
    let response: Value =
        serde_json::from_str(response).context("YouTube returned invalid playlist metadata")?;
    if let Some(error) = response.get("error") {
        bail!("YouTube playlist request failed: {}", error["message"]);
    }
    if let Some(alerts) = response["alerts"].as_array() {
        for alert in alerts {
            let alert = &alert["alertRenderer"];
            if alert["type"].as_str() == Some("ERROR") {
                bail!("YouTube playlist unavailable: {}", text(&alert["text"]));
            }
        }
    }
    let mut page = Page {
        visitor: response
            .pointer("/responseContext/visitorData")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ..Page::default()
    };
    if continuation {
        let commands = response
            .get("onResponseReceivedActions")
            .or_else(|| response.get("onResponseReceivedEndpoints"))
            .context("YouTube returned no playlist continuation items")?;
        collect(commands, &mut page);
    } else {
        let tabs = response
            .pointer("/contents/twoColumnBrowseResultsRenderer/tabs")
            .and_then(Value::as_array)
            .context("YouTube returned no playlist contents")?;
        let tab = tabs
            .iter()
            .find(|tab| tab["tabRenderer"]["selected"].as_bool() == Some(true))
            .or_else(|| {
                tabs.iter()
                    .find(|tab| tab["tabRenderer"].get("content").is_some())
            })
            .context("YouTube returned no playlist tab")?;
        collect(&tab["tabRenderer"]["content"], &mut page);
    }
    anyhow::ensure!(
        !page.entries.is_empty() || page.next.is_some(),
        "No accessible videos found in this YouTube playlist"
    );
    Ok(page)
}

pub async fn scan(
    record: &Record,
    rp: &RustyPipe,
    source: &url::Url,
    playlist: &str,
) -> Result<()> {
    let config = record.job.lock().await.config.clone();
    record
        .progress("Requesting YouTube playlist metadata", 0, None, "videos")
        .await;
    let checkpoint = record.dir.join("youtube-playlist-page.json");
    let mut page = if checkpoint.exists() {
        serde_json::from_slice(&tokio::fs::read(&checkpoint).await?)
            .context("Could not recover YouTube playlist scan")?
    } else {
        web::wait_saved_cooldown(record).await?;
        web::pace(source, Duration::from_millis(config.delay_ms)).await;
        parse(
            &rp.query()
                .raw(
                    ClientType::Desktop,
                    "browse",
                    &json!({"browseId": format!("VL{playlist}")}),
                )
                .await?,
            false,
        )?
    };
    loop {
        // Commit the page before adding its entries. Replaying it after a pause
        // is safe because entries are deduplicated by ID.
        use std::io::Write;
        let mut file = atomic_write_file::AtomicWriteFile::open(&checkpoint)?;
        file.write_all(&serde_json::to_vec(&page)?)?;
        file.commit()?;
        record
            .change(|job| {
                for entry in &page.entries {
                    if job.entries.iter().any(|existing| existing.id == entry.id) {
                        continue;
                    }
                    if job.entries.len() >= config.max_items {
                        job.scan_truncated = true;
                        break;
                    }
                    job.entries.push(entry.clone());
                }
                job.scan_truncated |= page.incomplete
                    || (job.entries.len() >= config.max_items && page.next.is_some());
                job.requests += 1;
                job.log(
                    "info",
                    format!(
                        "Discovered {} YouTube playlist videos locally.",
                        job.entries.len()
                    ),
                );
            })
            .await?;
        if record.job.lock().await.entries.len() >= config.max_items {
            break;
        }
        let Some(token) = &page.next else {
            break;
        };
        let discovered = record.job.lock().await.entries.len() as u64;
        record
            .progress(
                "Requesting the next playlist page",
                discovered,
                None,
                "videos",
            )
            .await;
        web::wait_saved_cooldown(record).await?;
        web::pace(source, Duration::from_millis(config.delay_ms)).await;
        let mut query = rp.query();
        if let Some(visitor) = &page.visitor {
            query = query.visitor_data(visitor);
        }
        let mut next = parse(
            &query
                .raw(
                    ClientType::Desktop,
                    "browse",
                    &json!({"continuation": token}),
                )
                .await?,
            true,
        )?;
        anyhow::ensure!(
            next.next.as_ref() != Some(token),
            "YouTube repeated a playlist page; pause and retry the scan later"
        );
        if next.visitor.is_none() {
            next.visitor = page.visitor;
        }
        page = next;
    }
    record.change(|job| {
        job.queue.clear();
        job.log("info", "Native YouTube playlist scan complete. Stream sizes are resolved when downloading.");
        if job.scan_truncated { job.log("warning", "Playlist scan is partial or reached the item limit. Review is required."); }
    }).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_current_lockups_classic_lists_and_continuations() {
        let lockup = json!({"lockupViewModel":{"contentType":"LOCKUP_CONTENT_TYPE_VIDEO","contentId":"O6VZaBM2Zr4","metadata":{"lockupMetadataViewModel":{"title":{"content":"A playlist video"}}},"contentImage":{"thumbnailViewModel":{"overlays":[{"thumbnailBadgeViewModel":{"text":"6:41"}}]}}}});
        let classic = json!({"playlistVideoRenderer":{"videoId":"secondvideo","title":{"runs":[{"text":"Classic video"}]},"lengthSeconds":"236"}});
        let next = json!({"continuationItemRenderer":{"continuationEndpoint":{"continuationCommand":{"token":"next-page"}}}});
        let response = json!({"responseContext":{"visitorData":"visitor"},"contents":{"twoColumnBrowseResultsRenderer":{"tabs":[{"tabRenderer":{"selected":true,"content":{"sectionListRenderer":{"contents":[{"itemSectionRenderer":{"contents":[]}},{"itemSectionRenderer":{"contents":[lockup, {"playlistVideoListRenderer":{"contents":[classic,next]}}]}}]}}}}]}},"sidebar":{"playlistVideoRenderer":{"videoId":"excluded","title":{"simpleText":"Recommendation"}}}});
        let page = parse(&response.to_string(), false).unwrap();
        assert_eq!(page.entries.len(), 2);
        assert_eq!(page.entries[0].duration, Some(401.0));
        assert_eq!(page.entries[1].duration, Some(236.0));
        assert_eq!(page.next.as_deref(), Some("next-page"));
        assert_eq!(page.visitor.as_deref(), Some("visitor"));
        assert!(!page.incomplete);
        let response = json!({"onResponseReceivedActions":[{"appendContinuationItemsAction":{"continuationItems":[{"playlistVideoRenderer":{"videoId":"private","title":{"simpleText":"[Private video]"},"isPlayable":false}}]}}]});
        let page = parse(&response.to_string(), true).unwrap();
        assert_eq!(page.entries.len(), 1);
        assert!(!page.entries[0].selected);
        assert_eq!(page.entries[0].state, "scan_failed");
        assert!(parse(r#"{"alerts":[{"alertRenderer":{"type":"ERROR","text":{"simpleText":"Playlist is private"}}}]}"#, false).unwrap_err().to_string().contains("Playlist is private"));
    }
}
