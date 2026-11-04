//! On-device media support. No Python runtime, child process, or remote service.
use crate::{
    model::*,
    web::{cooldown, http_url, key, pace, retry_delay, wait_saved_cooldown},
    Record,
};
use anyhow::{bail, Context, Result};
use rustypipe::{
    client::{ClientType, RustyPipe},
    model::{paginator::Paginator, UrlTarget, VideoItem, VideoPlayer},
};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

pub(crate) fn youtube(url: &url::Url) -> bool {
    matches!(
        url.host_str(),
        Some(
            "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtu.be"
        )
    )
}
pub fn supports(value: &str) -> bool {
    http_url(value)
        .map(|u| {
            youtube(&u)
                || ["mp4", "webm", "mp3", "m4a", "ogg", "wav", "mov"]
                    .iter()
                    .any(|ext| u.path().to_lowercase().ends_with(&format!(".{ext}")))
        })
        .unwrap_or(false)
}
pub(crate) fn youtube_thumbnail(value: &str) -> Option<String> {
    let url = http_url(value).ok()?;
    if !youtube(&url) {
        return None;
    }
    let id = if url.host_str() == Some("youtu.be") {
        url.path().trim_start_matches('/').to_owned()
    } else {
        url.query_pairs().find(|(k, _)| k == "v")?.1.into_owned()
    };
    (id.len() == 11
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')))
    .then(|| format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"))
}
fn client(record: &Record) -> Result<RustyPipe> {
    Ok(RustyPipe::builder()
        .storage_dir(record.dir.join("extractor"))
        .no_reporter()
        .timeout(Duration::from_secs(45))
        .n_http_retries(3)
        .build()?)
}
pub(crate) fn entry(
    id: &str,
    title: String,
    duration: Option<f64>,
    text: String,
    unavailable: bool,
) -> Entry {
    let url = format!("https://www.youtube.com/watch?v={id}");
    Entry {
        id: key(&url),
        url,
        title,
        depth: 0,
        selected: !unavailable,
        state: if unavailable {
            "scan_failed"
        } else {
            "pending"
        }
        .into(),
        size: None,
        duration,
        error: unavailable.then(|| "This video is unavailable, live, or upcoming".into()),
        text,
        assets: vec![],
        media_file: None,
        subtitles: vec![],
        transcript: vec![],
        transcript_file: None,
        subtitle_policy: None,
        subtitle_error: None,
        media_audio: None,
        thumbnail_url: Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")),
        thumbnail_file: None,
        embedded_in: vec![],
    }
}

pub async fn scan(record: &Record) -> Result<()> {
    let job = record.job.lock().await.clone();
    let source = http_url(&job.config.url)?;
    record
        .progress(
            "Getting media metadata",
            job.entries.len() as u64,
            None,
            "items",
        )
        .await;
    if !youtube(&source) {
        pace(&source, Duration::from_millis(job.config.delay_ms)).await;
        let response = reqwest::Client::new()
            .head(source.clone())
            .timeout(Duration::from_secs(45))
            .send()
            .await?;
        anyhow::ensure!(
            response.status().is_success(),
            "Media metadata request failed: {}",
            response.status()
        );
        let size = response.content_length();
        record
            .change(|j| {
                j.entries = vec![Entry {
                    id: key(source.as_str()),
                    url: source.to_string(),
                    title: source
                        .path_segments()
                        .and_then(|mut v| v.next_back())
                        .unwrap_or("Media file")
                        .into(),
                    depth: 0,
                    selected: true,
                    state: "pending".into(),
                    size,
                    duration: None,
                    error: None,
                    text: String::new(),
                    assets: vec![],
                    media_file: None,
                    subtitles: vec![],
                    transcript: vec![],
                    transcript_file: None,
                    subtitle_policy: None,
                    subtitle_error: None,
                    media_audio: None,
                    thumbnail_url: None,
                    thumbnail_file: None,
                    embedded_in: vec![],
                }];
                j.queue.clear();
                j.requests += 1;
            })
            .await?;
        return Ok(());
    }
    let rp = client(record)?;
    if let Some((_, playlist)) = source.query_pairs().find(|(key, _)| key == "list") {
        return crate::youtube_playlist::scan(record, &rp, &source, &playlist).await;
    }
    let cursor_path = record.dir.join("media-cursor.json");
    let mut page: Paginator<VideoItem> = if let Ok(data) = tokio::fs::read(&cursor_path).await {
        serde_json::from_slice(&data)?
    } else {
        pace(&source, Duration::from_millis(job.config.delay_ms)).await;
        if let Some(search) = source
            .query_pairs()
            .find(|(k, _)| k == "search_query")
            .map(|(_, v)| v.into_owned())
        {
            rp.query().search::<VideoItem, _>(search).await?.items
        } else if let Some(playlist) = source
            .query_pairs()
            .find(|(k, _)| k == "list")
            .map(|(_, v)| v.into_owned())
        {
            rp.query().playlist(playlist).await?.videos
        } else {
            match rp.query().resolve_url(source.as_str(), false).await? {
                UrlTarget::Video { id, .. } => {
                    let details = rp.query().video_details(&id).await?;
                    record
                        .change(|j| {
                            j.entries = vec![entry(
                                &id,
                                details.name,
                                None,
                                details
                                    .description
                                    .0
                                    .iter()
                                    .map(|c| c.get_text())
                                    .collect::<String>(),
                                details.is_live,
                            )];
                            j.queue.clear();
                        })
                        .await?;
                    return Ok(());
                }
                UrlTarget::Playlist { id } => rp.query().playlist(id).await?.videos,
                UrlTarget::Channel { id } => rp.query().channel_videos(id).await?.content,
                _ => bail!("This YouTube URL is not a video, playlist, channel, or search"),
            }
        }
    };
    loop {
        // Store the full current page before committing entries. Recovery deduplicates IDs.
        tokio::fs::write(&cursor_path, serde_json::to_vec(&page)?).await?;
        let entries = page
            .items
            .iter()
            .map(|v| {
                entry(
                    &v.id,
                    v.name.clone(),
                    v.duration.map(|v| v as f64),
                    v.short_description.clone().unwrap_or_default(),
                    v.is_live || v.is_upcoming,
                )
            })
            .collect::<Vec<_>>();
        let has_more = page.ctoken.is_some();
        record
            .change(|j| {
                for entry in entries {
                    if !j.entries.iter().any(|e| e.id == entry.id) {
                        if j.entries.len() >= j.config.max_items {
                            j.scan_truncated = true;
                            continue;
                        }
                        j.entries.push(entry);
                    }
                }
                if j.entries.len() >= j.config.max_items && has_more {
                    j.scan_truncated = true;
                }
                j.log(
                    "info",
                    format!("Discovered {} media items locally.", j.entries.len()),
                );
            })
            .await?;
        if record.job.lock().await.entries.len() >= job.config.max_items || page.ctoken.is_none() {
            break;
        }
        pace(&source, Duration::from_millis(job.config.delay_ms)).await;
        page = rp
            .query()
            .continuation(
                page.ctoken.as_deref().unwrap(),
                page.endpoint,
                page.visitor_data.as_deref(),
            )
            .await?;
    }
    record.change(|j| {
        j.queue.clear();
        if j.scan_truncated { j.log("warning", "Media scan reached the item limit. Review is required for this partial collection."); }
        j.log("info", "Native YouTube scan complete. Media sizes are unknown until streams are resolved after approval.");
    }).await?;
    anyhow::ensure!(
        !record.job.lock().await.entries.is_empty(),
        "No media items found"
    );
    Ok(())
}

pub(crate) async fn original_audio_player(
    record: &Record,
    rp: &RustyPipe,
    source: &url::Url,
    video: &str,
    delay_ms: u64,
) -> Result<VideoPlayer> {
    let clients = [ClientType::VisionOs, ClientType::Ios, ClientType::Android];
    let mut start = 0;
    loop {
        pace(source, Duration::from_millis(delay_ms)).await;
        let player = rp
            .query()
            .player_from_clients(video, &clients[start..])
            .await?;
        anyhow::ensure!(
            player.drm.is_none() && !player.details.is_live,
            "Protected and live streams are not supported"
        );
        match crate::youtube_audio::select(&player) {
            Ok(_) => return Ok(player),
            Err(error) => {
                start = clients
                    .iter()
                    .position(|c| *c == player.client_type)
                    .unwrap_or(clients.len())
                    + 1;
                if start >= clients.len() {
                    return Err(error.context("Could not identify playable original audio with the available player profiles"));
                }
                record
                    .log(
                        "warning",
                        format!(
                            "{error}. Trying another player profile; no dubbed audio was selected."
                        ),
                    )
                    .await?;
            }
        }
    }
}

pub async fn download(record: &Record) -> Result<()> {
    let job = record.job.lock().await.clone();
    let rp = job
        .entries
        .iter()
        .any(|e| http_url(&e.url).is_ok_and(|u| youtube(&u)))
        .then(|| client(record))
        .transpose()?;
    tokio::fs::create_dir_all(record.dir.join("media")).await?;
    for item in job.entries.iter().filter(|e| {
        crate::embedded::approved(e, &job)
            && e.state != "done"
            && (job.config.kind == SourceKind::Media || !e.embedded_in.is_empty())
    }) {
        let completed = record
            .job
            .lock()
            .await
            .entries
            .iter()
            .filter(|e| e.selected && e.state == "done")
            .count() as u64;
        let total = job.entries.iter().filter(|e| e.selected).count() as u64;
        record
            .progress(
                format!("Resolving streams: {}", item.title),
                completed,
                Some(total),
                "items",
            )
            .await;
        let result: Result<(String, Option<String>, u64)> = async {
            let source = http_url(&item.url)?;
            let mut user_agent = None;
            let (url, ext, identity, audio, description) = if let Some(rp) =
                rp.as_ref().filter(|_| youtube(&source))
            {
                let video = http_url(&item.url)?
                    .query_pairs()
                    .find(|(k, _)| k == "v")
                    .map(|(_, v)| v.into_owned())
                    .context("Missing video ID")?;
                let player =
                    original_audio_player(record, rp, &source, &video, job.config.delay_ms).await?;
                if !item.embedded_in.is_empty() {
                    record
                        .change(|j| {
                            let e = j.entries.iter_mut().find(|e| e.id == item.id).unwrap();
                            if let Some(title) = &player.details.name {
                                e.title = title.clone();
                            }
                            e.duration = Some(player.details.duration as f64);
                        })
                        .await?;
                }
                user_agent = Some(rp.query().user_agent(player.client_type).into_owned());
                let selected = crate::youtube_audio::select_with_height(
                    &player,
                    job.config.capture.video_quality.height(),
                )?;
                record
                    .log(
                        "info",
                        format!("{} · {}", item.title, selected.audio_note()),
                    )
                    .await?;
                if job.config.capture.subtitles {
                    crate::captions::save_sources(
                        record,
                        item,
                        crate::captions::Sources {
                            tracks: player.subtitles.clone(),
                            original_language: selected
                                .audio
                                .and_then(|a| a.track.as_ref())
                                .and_then(|t| t.lang.clone()),
                            resolved_at: crate::now(),
                            user_agent: Some(
                                rp.query().user_agent(player.client_type).into_owned(),
                            ),
                        },
                    )
                    .await?;
                }
                let stream = selected.video;
                let audio = selected.audio.map(|a| {
                    (
                        a.url.clone(),
                        "m4a".to_owned(),
                        crate::youtube_audio::audio_identity(&video, a),
                    )
                });
                let ext = if stream.mime.contains("webm") {
                    "webm"
                } else {
                    "mp4"
                };
                (
                    stream.url.clone(),
                    ext.to_string(),
                    format!("{video}:{}", stream.itag),
                    audio,
                    player.details.description.clone(),
                )
            } else {
                let ext = source
                    .path()
                    .rsplit('.')
                    .next()
                    .unwrap_or("mp4")
                    .to_lowercase();
                (item.url.clone(), ext, item.url.clone(), None, None)
            };
            let filename = format!("{}.{}", item.id, ext);
            let path = record.dir.join("media").join(&filename);
            record
                .progress(
                    format!("Downloading video: {}", item.title),
                    completed,
                    Some(total),
                    "items",
                )
                .await;
            record
                .log(
                    "info",
                    format!("Downloading {} on this device.", item.title),
                )
                .await?;
            let mut size = stream_file(
                record,
                &http_url(&url)?,
                &path,
                &identity,
                user_agent.as_deref(),
            )
            .await?;
            let mut audio_filename = None;
            if let Some((url, ext, identity)) = audio {
                let file = format!("{}-audio.{ext}", item.id);
                record
                    .progress(
                        format!("Downloading audio: {}", item.title),
                        completed,
                        Some(total),
                        "items",
                    )
                    .await;
                size += stream_file(
                    record,
                    &http_url(&url)?,
                    &record.dir.join("media").join(&file),
                    &identity,
                    user_agent.as_deref(),
                )
                .await?;
                audio_filename = Some(file);
            }
            if let Some(text) = description {
                record
                    .change(|j| {
                        j.entries.iter_mut().find(|e| e.id == item.id).unwrap().text = text;
                    })
                    .await?;
            }
            Ok((filename, audio_filename, size))
        }
        .await;
        match result {
            Ok((filename, audio, size)) => {
                record
                    .change(|j| {
                        let e = j.entries.iter_mut().find(|e| e.id == item.id).unwrap();
                        e.media_file = Some(filename);
                        e.media_audio = audio;
                        e.state = "done".into();
                        e.size = Some(size);
                        e.error = None;
                        j.bytes += size;
                        j.progress.completed = completed + 1;
                        j.progress.received = 0;
                        j.progress.expected = None;
                        j.progress.updated_at = now();
                        j.log("info", format!("Saved {}", item.title));
                    })
                    .await?
            }
            Err(error) => {
                record
                    .change(|j| {
                        let message = format!("{error:#}");
                        let e = j.entries.iter_mut().find(|e| e.id == item.id).unwrap();
                        e.state = "failed".into();
                        e.error = Some(message.clone());
                        j.log("error", format!("{}: {message}", item.title));
                    })
                    .await?;
                if error.is::<crate::capture::BudgetReached>()
                    || error.is::<crate::youtube_audio::OriginalAudioUnavailable>()
                {
                    return Err(error);
                }
                if error
                    .downcast_ref::<MediaHttpError>()
                    .is_some_and(|e| matches!(e.0, 401 | 403))
                {
                    return Err(error.context("The media server denied this request. Retry to resolve fresh stream URLs, or remove the unavailable item."));
                }
            }
        }
    }
    let failed = record
        .job
        .lock()
        .await
        .entries
        .iter()
        .filter(|e| crate::embedded::approved(e, &job) && e.state != "done")
        .count();
    anyhow::ensure!(
        failed == 0,
        "{failed} media items failed. Retry or remove them from the selection."
    );
    if let Some(rp) = &rp {
        crate::captions::prepare(record, rp).await?;
    }
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Validator {
    url: String,
    etag: String,
    #[serde(default)]
    total: Option<u64>,
}

#[derive(Debug)]
struct MediaHttpError(u16, String);
impl std::fmt::Display for MediaHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Media server {} returned HTTP {}", self.1, self.0)
    }
}
impl std::error::Error for MediaHttpError {}
async fn stream_file(
    record: &Record,
    source: &url::Url,
    path: &std::path::Path,
    identity: &str,
    user_agent: Option<&str>,
) -> Result<u64> {
    let youtube_cdn = source
        .host_str()
        .is_some_and(|host| host.ends_with(".googlevideo.com"));
    stream_file_with_policy(record, source, path, identity, user_agent, youtube_cdn).await
}

async fn stream_file_with_policy(
    record: &Record,
    source: &url::Url,
    path: &std::path::Path,
    identity: &str,
    user_agent: Option<&str>,
    youtube_cdn: bool,
) -> Result<u64> {
    let config = record.job.lock().await.config.clone();
    let cap = config.max_resource_mb * 1024 * 1024;
    let declared_size = youtube_cdn
        .then(|| {
            source
                .query_pairs()
                .find(|(key, _)| key == "clen")
                .and_then(|(_, value)| value.parse::<u64>().ok())
        })
        .flatten()
        .filter(|size| *size > 0);
    anyhow::ensure!(
        declared_size.unwrap_or(0) <= cap,
        "Media exceeds {} MB limit",
        config.max_resource_mb
    );
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        .build()?;
    let partial = path.with_extension("part");
    let meta = path.with_extension("part.json");
    let receipt = path.with_extension("complete.json");
    if let (Ok(data), Ok(file)) = (
        tokio::fs::read(&receipt).await,
        tokio::fs::metadata(path).await,
    ) {
        if let Ok((saved_identity, size)) = serde_json::from_slice::<(String, u64)>(&data) {
            if saved_identity == identity && size == file.len() && size > 0 {
                return Ok(size);
            }
        }
    }
    if record.remaining_download().await == Some(0) {
        return Err(crate::capture::BudgetReached.into());
    }
    for attempt in 0..=4 {
        wait_saved_cooldown(record).await?;
        let validator: Option<Validator> = tokio::fs::read(&meta)
            .await
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let existing = tokio::fs::metadata(&partial)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let mut offset = if validator
            .as_ref()
            .map(|v| {
                v.url == source.as_str()
                    && ((!v.etag.is_empty() && !v.etag.starts_with("W/"))
                        || (youtube_cdn && v.total.is_some()))
            })
            .unwrap_or(false)
        {
            existing
        } else {
            0
        };
        let mut resource_total = declared_size.or_else(|| {
            validator
                .as_ref()
                .filter(|v| v.url == source.as_str())
                .and_then(|v| v.total)
        });
        record.transfer_progress(offset, resource_total).await;
        let result: Result<u64> =
            async {
                loop {
                    pace(source, Duration::from_millis(config.delay_ms)).await;
                    let mut request_url = source.clone();
                    let mut requested_end = None;
                    let query_range = youtube_cdn && resource_total.is_some();
                    if query_range {
                        let end = resource_total
                            .unwrap()
                            .saturating_sub(1)
                            .min(offset.saturating_add(4 * 1024 * 1024 - 1));
                        let pairs: Vec<_> = source
                            .query_pairs()
                            .filter(|(key, _)| key != "range")
                            .map(|(k, v)| (k.into_owned(), v.into_owned()))
                            .collect();
                        request_url
                            .query_pairs_mut()
                            .clear()
                            .extend_pairs(pairs)
                            .append_pair("range", &format!("{offset}-{end}"));
                        requested_end = Some(end);
                    }
                    let mut request = client.get(request_url);
                    if let Some(agent) = user_agent {
                        request = request.header("User-Agent", agent);
                    }
                    if youtube_cdn {
                        // The CDN rejects ranges past the signed resource size.
                        let end = resource_total
                            .map(|total| total.saturating_sub(1))
                            .unwrap_or(offset.saturating_add(1023))
                            .min(offset.saturating_add(4 * 1024 * 1024 - 1));
                        if !query_range {
                            request = request.header("Range", format!("bytes={offset}-{end}"));
                        }
                        request = request
                            .header("Origin", "https://www.youtube.com")
                            .header("Referer", "https://www.youtube.com/");
                    }
                    if offset > 0 {
                        if !youtube_cdn {
                            request = request.header("Range", format!("bytes={offset}-"));
                        }
                        if let Some(v) = validator
                            .as_ref()
                            .filter(|v| !v.etag.is_empty() && !v.etag.starts_with("W/"))
                        {
                            request = request.header("If-Range", &v.etag);
                        }
                    }
                    record.change(|j| j.requests += 1).await?;
                    let response = request.send().await;
                    let mut response = response?;
                    let status = response.status();
                    if status.as_u16() == 429 || status.is_server_error() {
                        let delay = retry_delay(
                            response
                                .headers()
                                .get("retry-after")
                                .and_then(|h| h.to_str().ok()),
                            attempt,
                        );
                        cooldown(source, delay).await;
                        record
                            .change(|j| {
                                j.retry_at = Some(now() + delay.as_secs() + 1);
                                j.log(
                                    "warning",
                                    format!(
                                        "Media server returned {status}; waiting {} seconds.",
                                        delay.as_secs()
                                    ),
                                );
                            })
                            .await?;
                        tokio::time::sleep(delay).await;
                        record.change(|j| j.retry_at = None).await?;
                        bail!("Media server returned {status}");
                    }
                    if status.is_client_error() {
                        return Err(MediaHttpError(
                            status.as_u16(),
                            source.host_str().unwrap_or("unknown host").into(),
                        )
                        .into());
                    }
                    anyhow::ensure!(status.is_success(), "Media server returned {status}");
                    let append = (status.as_u16() == 206 || query_range) && offset > 0;
                    let mut full_size = None;
                    if status.as_u16() == 206 {
                        let range = response
                            .headers()
                            .get("content-range")
                            .and_then(|h| h.to_str().ok())
                            .context("Partial response lacks Content-Range")?;
                        let start = range
                            .strip_prefix("bytes ")
                            .and_then(|s| s.split('-').next())
                            .and_then(|s| s.parse::<u64>().ok())
                            .context("Invalid Content-Range")?;
                        anyhow::ensure!(start == offset, "Media server returned the wrong range");
                        full_size = range
                            .split('/')
                            .next_back()
                            .and_then(|v| v.parse::<u64>().ok());
                        anyhow::ensure!(
                            full_size.is_some(),
                            "Partial response lacks the complete file size"
                        );
                    }
                    let mut size = if append { offset } else { 0 };
                    let expected = response.content_length().map(|n| n + size);
                    let total = full_size
                        .or(if query_range { resource_total } else { None })
                        .or(expected);
                    resource_total = total;
                    anyhow::ensure!(
                        total.unwrap_or(0) <= cap,
                        "Media exceeds {} MB limit",
                        config.max_resource_mb
                    );
                    if record.remaining_download().await.is_some_and(|remaining| {
                        total.unwrap_or(0).saturating_sub(size) > remaining
                    }) {
                        return Err(crate::capture::BudgetReached.into());
                    }
                    let etag = response
                        .headers()
                        .get("etag")
                        .and_then(|h| h.to_str().ok())
                        .filter(|s| !s.starts_with("W/"));
                    if etag.is_some() || youtube_cdn {
                        tokio::fs::write(
                            &meta,
                            serde_json::to_vec(&Validator {
                                url: source.to_string(),
                                etag: etag.unwrap_or_default().to_string(),
                                total,
                            })?,
                        )
                        .await?;
                    } else {
                        let _ = tokio::fs::remove_file(&meta).await;
                    }
                    let mut file = tokio::fs::OpenOptions::new()
                        .create(true)
                        .write(true)
                        .truncate(!append)
                        .append(append)
                        .open(&partial)
                        .await?;
                    record.transfer_progress(size, total).await;
                    let mut last_update = std::time::Instant::now();
                    while let Some(chunk) = response.chunk().await? {
                        size += chunk.len() as u64;
                        anyhow::ensure!(size <= cap, "Media exceeded file-size limit");
                        record.consume_download(chunk.len() as u64).await?;
                        file.write_all(&chunk).await?;
                        if last_update.elapsed() >= Duration::from_millis(500) {
                            record.transfer_progress(size, total).await;
                            last_update = std::time::Instant::now();
                        }
                    }
                    file.sync_all().await?;
                    record.transfer_progress(size, total).await;
                    anyhow::ensure!(
                        size > 0 && expected.map(|n| n == size).unwrap_or(true),
                        "Media download was incomplete"
                    );
                    if let Some(end) = requested_end {
                        anyhow::ensure!(
                            size == end + 1,
                            "Media server did not return the requested byte range"
                        );
                    }
                    if total.is_some_and(|total| size < total) {
                        anyhow::ensure!(size > offset, "Media server returned an empty range");
                        offset = size;
                        continue;
                    }
                    anyhow::ensure!(
                        total.map(|total| total == size).unwrap_or(true),
                        "Media file size does not match the complete resource"
                    );
                    if path.exists() {
                        tokio::fs::remove_file(path).await?;
                    }
                    tokio::fs::rename(&partial, path).await?;
                    tokio::fs::write(&receipt, serde_json::to_vec(&(identity, size))?).await?;
                    let _ = tokio::fs::remove_file(&meta).await;
                    return Ok(size);
                }
            }
            .await;
        match result {
            Ok(size) => return Ok(size),
            Err(error) if error.is::<crate::capture::BudgetReached>() => return Err(error),
            Err(error) if error.downcast_ref::<MediaHttpError>().is_some() => return Err(error),
            Err(error) if attempt == 4 => return Err(error),
            Err(error) => {
                record.log("warning", format!("Media transfer interrupted: {error}. Retrying from a validated partial file where possible.")).await?;
                tokio::time::sleep(retry_delay(None, attempt)).await;
            }
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        extract::{OriginalUri, State},
        http::{HeaderMap, Method, StatusCode},
        response::Response,
        routing::any,
        Router,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[tokio::test]
    async fn replaces_legacy_or_dubbed_audio_receipts_and_reuses_verified_original() {
        let gets = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/audio.m4a",
                any(
                    |State(gets): State<Arc<AtomicUsize>>, method: Method| async move {
                        if method == Method::GET {
                            gets.fetch_add(1, Ordering::SeqCst);
                        }
                        Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Length", "5")
                            .header("ETag", "\"original\"")
                            .body(if method == Method::HEAD {
                                Body::empty()
                            } else {
                                Body::from("right")
                            })
                            .unwrap()
                    },
                ),
            )
            .with_state(gets.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let source = http_url(&format!("http://{addr}/audio.m4a")).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let engine = crate::Engine::open(temp.path()).await.unwrap();
        engine
            .update_settings(Settings {
                auto_start_small_jobs: false,
                ..Settings::default()
            })
            .await
            .unwrap();
        let job = engine
            .create(NewJob {
                url: source.to_string(),
                title: "Audio cache fixture".into(),
                kind: SourceKind::Media,
                max_items: 1,
                max_depth: 0,
                delay_ms: 1000,
                max_resource_mb: 1,
                language: "eng".into(),
                description: "Fixture".into(),
                creator: "Fixture".into(),
                include_assets: false,
                capture: Default::default(),
            })
            .await
            .unwrap();
        let record = engine.record(&job.id).await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while record.running.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let audio: rustypipe::model::AudioStream=serde_json::from_value(serde_json::json!({"url":source.as_str(),"itag":140,"bitrate":128000,"average_bitrate":128000,"size":5,"mime":"audio/mp4","format":"m4a","codec":"mp4a","track":{"id":"en.4","lang":"en","lang_name":"English original","is_default":true,"track_type":"original"},"drm_systems":[]})).unwrap();
        let original = crate::youtube_audio::audio_identity("fixture1234", &audio);
        let mut dubbed = audio.clone();
        dubbed.track.as_mut().unwrap().id = "ja.10".into();
        dubbed.track.as_mut().unwrap().lang = Some("ja".into());
        dubbed.track.as_mut().unwrap().track_type =
            Some(rustypipe::model::AudioTrackType::DubbedAuto);
        let path = record.dir.join("audio.m4a");
        let receipt = path.with_extension("complete.json");
        for stale in [
            "fixture1234:140".to_owned(),
            crate::youtube_audio::audio_identity("fixture1234", &dubbed),
        ] {
            tokio::fs::write(&path, b"wrong").await.unwrap();
            tokio::fs::write(&receipt, serde_json::to_vec(&(stale, 5u64)).unwrap())
                .await
                .unwrap();
            gets.store(0, Ordering::SeqCst);
            assert_eq!(
                stream_file_with_policy(&record, &source, &path, &original, None, false)
                    .await
                    .unwrap(),
                5
            );
            assert_eq!(tokio::fs::read(&path).await.unwrap(), b"right");
            assert_eq!(gets.load(Ordering::SeqCst), 1);
            stream_file_with_policy(&record, &source, &path, &original, None, false)
                .await
                .unwrap();
            assert_eq!(
                gets.load(Ordering::SeqCst),
                1,
                "Verified original audio should be reused on resume"
            );
        }
        server.abort();
    }

    #[tokio::test]
    async fn downloads_all_bounded_ranges_and_does_not_retry_forbidden() {
        let data = Arc::new(
            (0..(8 * 1024 * 1024 + 17))
                .map(|n| (n % 251) as u8)
                .collect::<Vec<_>>(),
        );
        let gets = Arc::new(AtomicUsize::new(0));
        let state = (data.clone(), gets.clone());
        let app = Router::new()
            .route(
                "/clip.mp4",
                any(
                    |State((data, gets)): State<(Arc<Vec<u8>>, Arc<AtomicUsize>)>,
                     method: Method,
                     headers: HeaderMap,
                     OriginalUri(uri): OriginalUri| async move {
                        if method == Method::HEAD {
                            return Response::builder()
                                .header("Content-Length", data.len())
                                .body(Body::empty())
                                .unwrap();
                        }
                        gets.fetch_add(1, Ordering::SeqCst);
                        let parsed = url::Url::parse(&format!("http://fixture{uri}")).unwrap();
                        let Some(range) = parsed
                            .query_pairs()
                            .find(|(key, _)| key == "range")
                            .map(|(_, value)| value.into_owned())
                        else {
                            return Response::builder()
                                .status(StatusCode::FORBIDDEN)
                                .body(Body::empty())
                                .unwrap();
                        };
                        assert_eq!(headers.get("user-agent").unwrap(), "ZimTest");
                        let (start, end) = range.split_once('-').unwrap();
                        let start = start.parse::<usize>().unwrap();
                        let end = end.parse::<usize>().unwrap();
                        if end >= data.len() {
                            return Response::builder()
                                .status(StatusCode::FORBIDDEN)
                                .body(Body::empty())
                                .unwrap();
                        }
                        Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Length", end - start + 1)
                            .body(Body::from(data[start..=end].to_vec()))
                            .unwrap()
                    },
                ),
            )
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let source = url::Url::parse(&format!(
            "http://{}/clip.mp4?clen={}",
            listener.local_addr().unwrap(),
            data.len()
        ))
        .unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let temp = tempfile::tempdir().unwrap();
        let engine = crate::Engine::open(temp.path()).await.unwrap();
        engine
            .update_settings(Settings {
                auto_start_small_jobs: false,
                ..Settings::default()
            })
            .await
            .unwrap();
        let job = engine
            .create(NewJob {
                url: source.to_string(),
                title: "Range fixture".into(),
                kind: SourceKind::Media,
                max_items: 1,
                max_depth: 0,
                delay_ms: 1000,
                max_resource_mb: 16,
                language: "eng".into(),
                description: "Range test".into(),
                creator: "Test".into(),
                include_assets: false,
                capture: Default::default(),
            })
            .await
            .unwrap();
        let record = engine.record(&job.id).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while record.running.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let path = record.dir.join("fixture.mp4");
        let size =
            stream_file_with_policy(&record, &source, &path, "fixture", Some("ZimTest"), true)
                .await
                .unwrap();
        assert_eq!(size, data.len() as u64);
        assert_eq!(tokio::fs::read(&path).await.unwrap(), *data);
        assert_eq!(gets.load(Ordering::SeqCst), 3);
        assert_eq!(record.job.lock().await.progress.received, size);
        assert_eq!(record.job.lock().await.progress.expected, Some(size));
        let denied = stream_file_with_policy(
            &record,
            &source,
            &record.dir.join("denied.mp4"),
            "denied",
            None,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(denied.downcast_ref::<MediaHttpError>().unwrap().0, 403);
        assert_eq!(
            gets.load(Ordering::SeqCst),
            4,
            "403 must not be retried unchanged"
        );
        server.abort();
    }

    #[tokio::test]
    #[ignore = "Downloads one complete YouTube video and audio track, then builds a ZIM"]
    async fn downloads_youtube_video_and_builds_zim() {
        let temp = tempfile::tempdir().unwrap();
        let engine = crate::Engine::open(temp.path()).await.unwrap();
        engine
            .update_settings(Settings {
                auto_start_small_jobs: false,
                ..Settings::default()
            })
            .await
            .unwrap();
        let job = engine
            .create(NewJob {
                url: std::env::var("ZIMSMITH_TEST_VIDEO_URL")
                    .unwrap_or_else(|_| "https://www.youtube.com/watch?v=O6VZaBM2Zr4".into()),
                title: "YouTube native transfer check".into(),
                kind: SourceKind::Media,
                max_items: 1,
                max_depth: 0,
                delay_ms: 1000,
                max_resource_mb: 256,
                language: "eng".into(),
                description: "Native transfer validation".into(),
                creator: "Zim Studio test".into(),
                include_assets: false,
                capture: Default::default(),
            })
            .await
            .unwrap();
        let record = engine.record(&job.id).await.unwrap();
        tokio::time::timeout(Duration::from_secs(90), async {
            while record.running.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(engine.get(&job.id).await.unwrap().status, "review");
        record
            .change(|j| {
                j.phase = "download".into();
                j.status = "downloading".into();
            })
            .await
            .unwrap();
        let transfer = tokio::time::timeout(Duration::from_secs(300), download(&record))
            .await
            .unwrap();
        if transfer.is_err() {
            let state = engine.get(&job.id).await.unwrap();
            println!("Failed stage: {:?}", state.progress);
            for log in &state.logs {
                println!("{}: {}", log.level, log.message);
            }
        }
        transfer.unwrap();
        if let Ok(destination) = std::env::var("ZIMSTUDIO_SYNC_FIXTURE_DIR") {
            tokio::fs::create_dir_all(&destination).await.unwrap();
            let saved = engine.get(&job.id).await.unwrap();
            let entry = &saved.entries[0];
            for (file, name) in [
                (entry.media_file.as_ref(), "video.mp4"),
                (entry.media_audio.as_ref(), "audio.m4a"),
            ] {
                if let Some(file) = file {
                    tokio::fs::copy(
                        record.dir.join("media").join(file),
                        std::path::Path::new(&destination).join(name),
                    )
                    .await
                    .unwrap();
                }
            }
        }
        record
            .change(|j| {
                j.phase = "package".into();
                j.status = "packaging".into();
            })
            .await
            .unwrap();
        crate::archive::package(&record).await.unwrap();
        let complete = engine.get(&job.id).await.unwrap();
        assert_eq!(complete.status, "completed");
        assert_eq!(complete.entries[0].state, "done");
        assert!(complete.entries[0].size.unwrap() > 1024);
        let archive = zimru::Archive::open(engine.artifact(&job.id).await.unwrap()).unwrap();
        assert!(archive
            .get_text("index.html")
            .unwrap()
            .contains(&crate::archive::escape(&complete.entries[0].title)));
        let entry = &complete.entries[0];
        let page = archive.get_text(&format!("{}.html", entry.id)).unwrap();
        assert_eq!(page.matches("<video ").count(), 1);
        assert!(!page.contains("audio-track") && !page.contains("player.js"));
        if entry.media_audio.is_some() {
            assert!(
                archive
                    .get_bytes(&format!("{}-combined.mp4", entry.id))
                    .unwrap()
                    .len()
                    > 1024
            );
        }
        if let Ok(destination) = std::env::var("ZIMSTUDIO_SYNC_FIXTURE_DIR") {
            tokio::fs::copy(
                engine.artifact(&job.id).await.unwrap(),
                std::path::Path::new(&destination).join("archive.zim"),
            )
            .await
            .unwrap();
        }
        if let Ok(expected) = std::env::var("ZIMSMITH_EXPECT_AUDIO_LANGUAGE") {
            assert!(complete.logs.iter().any(|l| l.message.contains("Audio:")
                && l.message.contains(&expected)
                && l.message.contains("original")));
        }
        if std::env::var_os("ZIMSMITH_TEST_AUDIO_REPAIR").is_some() {
            let entry = complete.entries[0].clone();
            let video_path = record
                .dir
                .join("media")
                .join(entry.media_file.as_ref().unwrap());
            let modified = tokio::fs::metadata(&video_path)
                .await
                .unwrap()
                .modified()
                .unwrap();
            let audio_path = record
                .dir
                .join("media")
                .join(entry.media_audio.as_ref().expect("Separate original audio"));
            let size = tokio::fs::metadata(&audio_path).await.unwrap().len();
            tokio::fs::write(
                audio_path.with_extension("complete.json"),
                serde_json::to_vec(&("legacy-itag-only", size)).unwrap(),
            )
            .await
            .unwrap();
            engine.repair_youtube_audio(&job.id).await.unwrap();
            tokio::time::timeout(Duration::from_secs(300), async {
                while record.running.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            })
            .await
            .unwrap();
            let repaired = engine.get(&job.id).await.unwrap();
            assert_eq!(repaired.status, "completed", "{:?}", repaired.error);
            assert_eq!(repaired.archive_revision, complete.archive_revision + 1);
            assert_eq!(
                tokio::fs::metadata(&video_path)
                    .await
                    .unwrap()
                    .modified()
                    .unwrap(),
                modified,
                "Matching video should not be downloaded again"
            );
            let receipt: (String, u64) = serde_json::from_slice(
                &tokio::fs::read(audio_path.with_extension("complete.json"))
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert!(receipt.0.starts_with("youtube-audio-v2:"));
            println!("Live original-audio repair rebuilt the ZIM and reused the saved video.");
        }
        println!(
            "Completed native video+audio download and valid ZIM: {} bytes",
            complete.output_bytes.unwrap()
        );
    }
}
