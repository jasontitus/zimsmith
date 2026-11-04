use crate::{
    model::*,
    web::{http_url, key},
    Record,
};
use anyhow::{Context, Result};
use serde_json::Value;
use std::time::Duration;
use tokio::process::Command;

fn executable() -> Command {
    if let Ok(path) = std::env::var("ZIMSTUDIO_YTDLP") {
        return Command::new(path);
    }
    let source = std::env::var_os("ZIMSTUDIO_YTDLP_SOURCE")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            // Only development builds know the source checkout. Never embed
            // its absolute path in a distributed app.
            #[cfg(debug_assertions)]
            {
                Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tools/yt-dlp"))
            }
            #[cfg(not(debug_assertions))]
            {
                None
            }
        });
    if let Some(source) = source.filter(|p| p.join("yt_dlp/__init__.py").exists()) {
        let python = std::env::var("ZIMSTUDIO_PYTHON")
            .unwrap_or_else(|_| if cfg!(windows) { "python" } else { "python3" }.into());
        let mut command = Command::new(python);
        command.args(["-m", "yt_dlp"]).env("PYTHONPATH", source);
        command
    } else {
        Command::new("yt-dlp")
    }
}

pub async fn probe(name: &str, flag: &str) -> ToolStatus {
    if cfg!(any(target_os = "ios", target_os = "android")) {
        return ToolStatus { available: false, detail: "Native YouTube and direct-file extraction are included; yt-dlp is an optional desktop adapter".into() };
    }
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        (if name == "yt-dlp" {
            executable()
        } else {
            Command::new(name)
        })
        .arg(flag)
        .kill_on_drop(true)
        .output(),
    )
    .await;
    match result {
        Ok(Ok(out)) if out.status.success() => ToolStatus {
            available: true,
            detail: String::from_utf8_lossy(&out.stdout)
                .trim()
                .chars()
                .take(160)
                .collect(),
        },
        Ok(Ok(out)) => ToolStatus {
            available: false,
            detail: String::from_utf8_lossy(&out.stderr)
                .trim()
                .chars()
                .take(600)
                .collect(),
        },
        Ok(Err(error)) => ToolStatus {
            available: false,
            detail: error.to_string(),
        },
        Err(_) => ToolStatus {
            available: false,
            detail: "Tool did not respond within 15 seconds".into(),
        },
    }
}

fn command(delay_ms: u64) -> Command {
    let mut command = executable();
    command
        .kill_on_drop(true)
        .args([
            "--ignore-config",
            "--no-plugin-dirs",
            "--no-progress",
            "--no-colors",
            "--socket-timeout",
            "30",
            "--retries",
            "4",
            "--extractor-retries",
            "4",
            "--fragment-retries",
            "4",
            "--retry-sleep",
            "http:exp=5:120",
            "--retry-sleep",
            "extractor:exp=5:120",
            "--retry-sleep",
            "fragment:exp=5:120",
            "--concurrent-fragments",
            "1",
            "--abort-on-unavailable-fragments",
            "--sleep-requests",
        ])
        .arg(format!("{:.2}", delay_ms as f64 / 1000.0));
    command
}

pub async fn scan(record: &Record) -> Result<()> {
    let native = super::native_media::supports(&record.job.lock().await.config.url);
    if native {
        match super::native_media::scan(record).await {
            Ok(()) => return Ok(()),
            Err(error) if cfg!(any(target_os = "ios", target_os = "android")) => return Err(error),
            Err(error) => {
                record
                    .log(
                        "warning",
                        format!("Native extractor failed: {error}. Trying local yt-dlp."),
                    )
                    .await?
            }
        }
    }
    anyhow::ensure!(!cfg!(any(target_os = "ios", target_os = "android")), "This provider needs an on-device extractor that is not yet included. YouTube channels, playlists, search URLs and direct media files are supported natively.");
    let job = record.job.lock().await.clone();
    let out = tokio::time::timeout(
        Duration::from_secs(1800),
        command(job.config.delay_ms)
            .args([
                "--flat-playlist",
                "--dump-single-json",
                "--skip-download",
                "--playlist-end",
            ])
            .arg(job.config.max_items.to_string())
            .arg("--")
            .arg(&job.config.url)
            .output(),
    )
    .await
    .context("Media scan timed out; resume to retry")?
    .context("Could not start yt-dlp. Install it or configure ZIMSTUDIO_YTDLP")?;
    anyhow::ensure!(
        out.status.success(),
        "yt-dlp scan failed: {}",
        tail(&out.stderr)
    );
    let value: Value =
        serde_json::from_slice(&out.stdout).context("yt-dlp returned invalid metadata")?;
    let values: Vec<&Value> = value
        .get("entries")
        .and_then(Value::as_array)
        .map(|v| v.iter().collect())
        .unwrap_or_else(|| vec![&value]);
    let mut entries = Vec::new();
    let mut incomplete = value.get("entries").is_some() && values.len() >= job.config.max_items;
    for value in values.into_iter().take(job.config.max_items) {
        let Some(url) = media_url(value) else {
            incomplete = true;
            continue;
        };
        if entries.iter().any(|e: &Entry| e.url == url) {
            continue;
        }
        let title = value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Untitled media")
            .to_string();
        let unavailable = value
            .get("availability")
            .and_then(Value::as_str)
            .map(|s| ["private", "premium_only", "subscriber_only", "needs_auth"].contains(&s))
            .unwrap_or(false)
            || ["[Deleted video]", "[Private video]"].contains(&title.as_str());
        entries.push(Entry {
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
            size: value
                .get("filesize")
                .or_else(|| value.get("filesize_approx"))
                .and_then(Value::as_u64),
            duration: value.get("duration").and_then(Value::as_f64),
            error: unavailable.then(|| "Media is unavailable or requires authentication".into()),
            text: value
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(100_000)
                .collect(),
            assets: vec![],
            media_file: None,
            subtitles: vec![],
            transcript: vec![],
            transcript_file: None,
            subtitle_policy: None,
            subtitle_error: None,
            media_audio: None,
            thumbnail_url: value
                .get("thumbnail")
                .and_then(Value::as_str)
                .or_else(|| {
                    value
                        .get("thumbnails")?
                        .as_array()?
                        .last()?
                        .get("url")?
                        .as_str()
                })
                .map(str::to_owned),
            thumbnail_file: None,
            embedded_in: vec![],
            rendered: None,
        });
    }
    anyhow::ensure!(
        entries.iter().any(|e| e.state != "scan_failed"),
        "No downloadable media entries found"
    );
    record.change(|j| {
        j.entries = entries; j.queue.clear(); j.scan_truncated = incomplete || !out.stderr.is_empty();
        if !out.stderr.is_empty() { j.log("warning", tail(&out.stderr)); }
        j.log("info", "Flat media scan complete. Sizes may be unknown until download. yt-dlp manages extractor requests and retries; website robots rules apply to the website crawler.");
        if incomplete { j.log("warning", "Media scan may be incomplete or reached the item limit. Review is required."); }
    }).await
}

fn media_url(value: &Value) -> Option<String> {
    for field in ["webpage_url", "original_url", "url"] {
        if let Some(url) = value
            .get(field)
            .and_then(Value::as_str)
            .and_then(|s| http_url(s).ok())
        {
            return Some(url.to_string());
        }
    }
    let extractor = value
        .get("ie_key")
        .or_else(|| value.get("extractor_key"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    if extractor.contains("youtube") {
        let id = value.get("id").and_then(Value::as_str)?;
        if id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Some(format!("https://www.youtube.com/watch?v={id}"));
        }
    }
    None
}

pub async fn download(record: &Record) -> Result<()> {
    let native = super::native_media::supports(&record.job.lock().await.config.url);
    if native {
        match super::native_media::download(record).await {
            Ok(()) => return Ok(()),
            Err(error)
                if cfg!(any(target_os = "ios", target_os = "android"))
                    || error.is::<crate::youtube_audio::OriginalAudioUnavailable>()
                    || error.is::<crate::capture::BudgetReached>() =>
            {
                return Err(error)
            }
            Err(error) => {
                record
                    .log(
                        "warning",
                        format!(
                        "Native download failed: {error}. Trying local yt-dlp for remaining items."
                    ),
                    )
                    .await?
            }
        }
    }
    let snapshot = record.job.lock().await.clone();
    let media_dir = record.dir.join("media");
    tokio::fs::create_dir_all(&media_dir).await?;
    for entry in snapshot
        .entries
        .iter()
        .filter(|e| e.selected && e.state != "done")
    {
        let completed = record
            .job
            .lock()
            .await
            .entries
            .iter()
            .filter(|e| e.selected && e.state == "done")
            .count() as u64;
        let total = snapshot.entries.iter().filter(|e| e.selected).count() as u64;
        record
            .progress(
                format!("Downloading {} with yt-dlp", entry.title),
                completed,
                Some(total),
                "items",
            )
            .await;
        record
            .log("info", format!("Downloading {}", entry.title))
            .await?;
        let result: Result<(String, u64)> = async {
            let remaining = record.remaining_download().await.unwrap_or(u64::MAX);
            let cap = remaining.min(snapshot.config.max_resource_mb * 1024 * 1024);
            anyhow::ensure!(cap > 0, crate::capture::BudgetReached);
            let format = format!(
                "best[ext=mp4][height<={}]/best[height<={}]",
                snapshot.config.capture.video_quality.height(),
                snapshot.config.capture.video_quality.height()
            );
            let suffix = if snapshot.archive_revision == 0 {
                String::new()
            } else {
                format!("-r{}", snapshot.archive_revision)
            };
            let out = tokio::time::timeout(
                Duration::from_secs(12 * 3600),
                command(snapshot.config.delay_ms)
                    .args([
                        "--no-playlist",
                        "--continue",
                        "--no-overwrites",
                        "--restrict-filenames",
                        "--format-sort",
                        "lang",
                        "--format",
                        &format,
                        "--max-filesize",
                    ])
                    .arg(cap.to_string())
                    .args(["--print", "after_move:filepath", "--output"])
                    .arg(media_dir.join(format!("{}-ytdlp-v2{suffix}.%(ext)s", entry.id)))
                    .arg("--")
                    .arg(&entry.url)
                    .output(),
            )
            .await
            .context("Media download timed out")?
            .context("Could not launch yt-dlp")?;
            anyhow::ensure!(out.status.success(), "{}", tail(&out.stderr));
            let stdout = String::from_utf8_lossy(&out.stdout);
            let path = stdout
                .lines()
                .last()
                .context("No output file was produced (media may exceed size limit)")?;
            let actual = std::path::Path::new(path);
            let canonical = tokio::fs::canonicalize(actual).await?;
            anyhow::ensure!(
                canonical.starts_with(tokio::fs::canonicalize(&media_dir).await?),
                "Unexpected media output path"
            );
            let size = tokio::fs::metadata(&canonical).await?.len();
            anyhow::ensure!(
                size <= snapshot.config.max_resource_mb * 1024 * 1024,
                "Media exceeds the configured size limit"
            );
            record.consume_download(size).await?;
            let filename = canonical
                .file_name()
                .context("Missing media filename")?
                .to_string_lossy()
                .into_owned();
            Ok((filename, size))
        }
        .await;
        match result {
            Ok((filename, size)) => {
                record
                    .change(|j| {
                        let e = j.entries.iter_mut().find(|e| e.id == entry.id).unwrap();
                        e.state = "done".into();
                        e.media_file = Some(filename);
                        e.media_audio = None;
                        e.size = Some(size);
                        e.error = None;
                        j.bytes += size;
                        j.log("info", format!("Downloaded {}", entry.title));
                    })
                    .await?
            }
            Err(error) => {
                record
                    .change(|j| {
                        let message = format!("{error:#}");
                        let e = j.entries.iter_mut().find(|e| e.id == entry.id).unwrap();
                        e.state = "failed".into();
                        e.error = Some(message.clone());
                        j.log("error", format!("{}: {message}", entry.title));
                    })
                    .await?
            }
        }
    }
    let failed = record
        .job
        .lock()
        .await
        .entries
        .iter()
        .filter(|e| e.selected && e.state != "done")
        .count();
    anyhow::ensure!(
        failed == 0,
        "{failed} media items failed. Resume to retry or remove failed items from the selection."
    );
    Ok(())
}

fn tail(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(4000)..]).into_owned()
}
