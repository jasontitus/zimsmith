use crate::{
    capture::BudgetReached,
    web::{cooldown, http_url, pace, retry_delay, wait_saved_cooldown},
    Entry, Record,
};
use anyhow::{Context, Result};
use rustypipe::{client::RustyPipe, model::Subtitle};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cue {
    pub start: f64,
    pub duration: f64,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedSubtitle {
    pub file: String,
    pub language: String,
    pub label: String,
    pub auto_generated: bool,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Sources {
    pub tracks: Vec<Subtitle>,
    pub original_language: Option<String>,
    pub resolved_at: u64,
    #[serde(default)]
    pub user_agent: Option<String>,
}

pub fn parse_json3(bytes: &[u8]) -> Result<Vec<Cue>> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).context("Caption response was not JSON3")?;
    let events = value
        .get("events")
        .and_then(|v| v.as_array())
        .context("Caption response has no events")?;
    anyhow::ensure!(
        events.len() <= 100_000,
        "Caption response has too many events"
    );
    let mut cues = Vec::new();
    for event in events {
        let number = |name: &str| {
            event
                .get(name)
                .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
        };
        let Some(start) = number("tStartMs") else {
            continue;
        };
        let duration = number("dDurationMs").unwrap_or(3000.0);
        if !start.is_finite() || start < 0.0 || !duration.is_finite() || duration <= 0.0 {
            continue;
        }
        let text = event
            .get("segs")
            .and_then(|v| v.as_array())
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p.get("utf8").and_then(|v| v.as_str()))
                    .collect::<String>()
            })
            .unwrap_or_default();
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        anyhow::ensure!(text.len() <= 20_000, "Caption event exceeds safe text size");
        cues.push(Cue {
            start: start / 1000.0,
            duration: duration / 1000.0,
            text,
        });
    }
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    cues.dedup_by(|a, b| a.start == b.start && a.text == b.text);
    anyhow::ensure!(
        !cues.is_empty(),
        "Caption response contained no usable text"
    );
    Ok(cues)
}
fn timestamp(seconds: f64) -> String {
    let ms = (seconds * 1000.0).max(0.0).min(u64::MAX as f64) as u64;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3600000,
        (ms / 60000) % 60,
        (ms / 1000) % 60,
        ms % 1000
    )
}
pub fn vtt(cues: &[Cue]) -> String {
    let mut result = String::from("WEBVTT\n\n");
    for cue in cues {
        result.push_str(&format!(
            "{} --> {}\n{}\n\n",
            timestamp(cue.start),
            timestamp(cue.start + cue.duration),
            crate::archive::escape(&cue.text)
        ));
    }
    result
}
pub(crate) fn choose<'a>(
    sources: &'a Sources,
    requested: &str,
    auto: bool,
) -> Result<&'a Subtitle> {
    let original = sources.original_language.as_deref().or_else(|| {
        sources
            .tracks
            .iter()
            .find(|s| s.auto_generated)
            .map(|s| s.lang.as_str())
    });
    let target = if requested == "original" {
        original.or_else(|| {
            let first=sources.tracks.first()?;
            sources.tracks.iter().all(|s| s.lang.eq_ignore_ascii_case(&first.lang)).then_some(first.lang.as_str())
        }).context("Original caption language is not identified. Choose an explicit subtitle language.")?
    } else {
        requested
    };
    sources
        .tracks
        .iter()
        .filter(|s| auto || !s.auto_generated)
        .filter(|s| {
            s.lang.eq_ignore_ascii_case(target)
                || s.lang
                    .split('-')
                    .next()
                    .unwrap_or("")
                    .eq_ignore_ascii_case(target.split('-').next().unwrap_or(""))
        })
        .max_by_key(|s| (!s.auto_generated, s.lang.eq_ignore_ascii_case(target)))
        .context("No captions are available in the requested language")
}
#[derive(Debug)]
struct CaptionHttp(u16);
impl std::fmt::Display for CaptionHttp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Caption server returned HTTP {}", self.0)
    }
}
impl std::error::Error for CaptionHttp {}

async fn fetch(record: &Record, track: &Subtitle, user_agent: Option<&str>) -> Result<Vec<Cue>> {
    let mut url = http_url(&track.url)?;
    let pairs: Vec<_> = url
        .query_pairs()
        .filter(|(k, _)| k != "fmt")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.query_pairs_mut()
        .clear()
        .extend_pairs(pairs)
        .append_pair("fmt", "json3");
    let delay = Duration::from_millis(record.job.lock().await.config.delay_ms);
    wait_saved_cooldown(record).await?;
    if record.remaining_download().await == Some(0) {
        return Err(BudgetReached.into());
    }
    pace(&url, delay).await;
    record.change(|j| j.requests += 1).await?;
    let mut response = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?
        .get(url.clone())
        .header(
            "User-Agent",
            user_agent.unwrap_or("Zimsmith offline archiver"),
        )
        .send()
        .await?;
    cooldown(&url, delay).await;
    let status = response.status();
    if status.as_u16() == 429 || status.is_server_error() {
        let wait = retry_delay(
            response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok()),
            0,
        );
        cooldown(&url, wait).await;
        record
            .change(|j| j.retry_at = Some(crate::now() + wait.as_secs() + 1))
            .await?;
    }
    if !status.is_success() {
        return Err(CaptionHttp(status.as_u16()).into());
    }
    anyhow::ensure!(
        response.content_length().unwrap_or(0) <= 2 * 1024 * 1024,
        "Caption file exceeds 2 MB limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= 2 * 1024 * 1024,
            "Caption file exceeds 2 MB limit"
        );
        record.consume_download(chunk.len() as u64).await?;
        bytes.extend_from_slice(&chunk);
        record
            .transfer_progress(bytes.len() as u64, response.content_length())
            .await;
    }
    parse_json3(&bytes)
}
pub(crate) async fn save_sources(record: &Record, entry: &Entry, sources: Sources) -> Result<()> {
    tokio::fs::write(
        record
            .dir
            .join("media")
            .join(format!("{}-caption-sources.json", entry.id)),
        serde_json::to_vec(&sources)?,
    )
    .await?;
    Ok(())
}
async fn sources(record: &Record, entry: &Entry, rp: &RustyPipe, fresh: bool) -> Result<Sources> {
    let path = record
        .dir
        .join("media")
        .join(format!("{}-caption-sources.json", entry.id));
    if !fresh {
        if let Ok(bytes) = tokio::fs::read(&path).await {
            if let Ok(sources) = serde_json::from_slice::<Sources>(&bytes) {
                if crate::now().saturating_sub(sources.resolved_at) < 1800 {
                    return Ok(sources);
                }
            }
        }
    }
    let url = http_url(&entry.url)?;
    let video = url
        .query_pairs()
        .find(|(k, _)| k == "v")
        .context("Missing video ID")?
        .1
        .into_owned();
    let delay = record.job.lock().await.config.delay_ms;
    let player =
        crate::native_media::original_audio_player(record, rp, &url, &video, delay).await?;
    let selected = crate::youtube_audio::select(&player)?;
    let original_language = selected
        .audio
        .and_then(|a| a.track.as_ref())
        .and_then(|t| t.lang.clone());
    let sources = Sources {
        tracks: player.subtitles,
        original_language,
        resolved_at: crate::now(),
        user_agent: Some(rp.query().user_agent(player.client_type).into_owned()),
    };
    save_sources(
        record,
        entry,
        Sources {
            tracks: sources.tracks.clone(),
            original_language: sources.original_language.clone(),
            resolved_at: sources.resolved_at,
            user_agent: sources.user_agent.clone(),
        },
    )
    .await?;
    Ok(sources)
}
pub(crate) async fn prepare(record: &Record, rp: &RustyPipe) -> Result<()> {
    let job = record.job.lock().await.clone();
    if !job.config.capture.subtitles {
        return Ok(());
    }
    let policy = format!(
        "{}:{}",
        job.config.capture.subtitle_language, job.config.capture.auto_captions
    );
    for entry in job
        .entries
        .iter()
        .filter(|e| e.selected && e.state == "done")
    {
        if entry.subtitle_policy.as_deref() == Some(&policy) && !entry.subtitles.is_empty() {
            let mut valid = true;
            for track in &entry.subtitles {
                valid &= tokio::fs::metadata(record.dir.join("media").join(&track.file))
                    .await
                    .is_ok_and(|m| m.is_file());
            }
            valid &= if let Some(file) = &entry.transcript_file {
                tokio::fs::metadata(record.dir.join("media").join(file))
                    .await
                    .is_ok_and(|m| m.is_file())
            } else {
                false
            };
            if valid {
                continue;
            }
        }
        {
            record
                .change(|j| {
                    let e = j.entries.iter_mut().find(|e| e.id == entry.id).unwrap();
                    e.subtitles.clear();
                    e.transcript.clear();
                    e.transcript_file = None;
                    e.subtitle_policy = None;
                })
                .await?;
        }
        record
            .progress(
                format!("Saving captions: {}", entry.title),
                0,
                None,
                "tracks",
            )
            .await;
        let result: Result<(SavedSubtitle, Vec<Cue>)> = async {
            let mut available = sources(record, entry, rp, false).await?;
            let mut track = choose(
                &available,
                &job.config.capture.subtitle_language,
                job.config.capture.auto_captions,
            )?;
            let cues = match fetch(record, track, available.user_agent.as_deref()).await {
                Ok(cues) => cues,
                Err(error)
                    if error
                        .downcast_ref::<CaptionHttp>()
                        .is_some_and(|e| matches!(e.0, 401 | 403)) =>
                {
                    available = sources(record, entry, rp, true).await?;
                    track = choose(
                        &available,
                        &job.config.capture.subtitle_language,
                        job.config.capture.auto_captions,
                    )?;
                    fetch(record, track, available.user_agent.as_deref()).await?
                }
                Err(error) => return Err(error),
            };
            let file = format!("{}-captions.vtt", entry.id);
            tokio::fs::write(record.dir.join("media").join(&file), vtt(&cues)).await?;
            Ok((
                SavedSubtitle {
                    file,
                    language: track.lang.clone(),
                    label: format!(
                        "{}{}",
                        track.lang_name,
                        if track.auto_generated {
                            " (auto-generated)"
                        } else {
                            ""
                        }
                    ),
                    auto_generated: track.auto_generated,
                },
                cues,
            ))
        }
        .await;
        match result {
            Ok((subtitle, cues)) => {
                let transcript_file = format!("{}-transcript.json", entry.id);
                tokio::fs::write(
                    record.dir.join("media").join(&transcript_file),
                    serde_json::to_vec(&cues)?,
                )
                .await?;
                record
                    .change(|j| {
                        let entry = j.entries.iter_mut().find(|e| e.id == entry.id).unwrap();
                        entry.subtitles = vec![subtitle];
                        entry.transcript.clear();
                        entry.transcript_file = Some(transcript_file);
                        entry.subtitle_error = None;
                        entry.subtitle_policy = Some(policy.clone());
                    })
                    .await?
            }
            Err(error) => {
                let budget = error.is::<BudgetReached>();
                record
                    .change(|j| {
                        let e = j.entries.iter_mut().find(|e| e.id == entry.id).unwrap();
                        e.subtitle_error = Some(format!("{error:#}"));
                        j.log(
                            "warning",
                            format!("Captions unavailable for {}: {error}", entry.title),
                        );
                    })
                    .await?;
                if budget {
                    return Err(error);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json3_produces_safe_timed_vtt_and_rejects_empty_responses() {
        let cues=parse_json3(br#"{"events":[{"tStartMs":1250,"dDurationMs":2500,"segs":[{"utf8":"<script> & hello"}]},{"tStartMs":5000,"segs":[{"utf8":"  next\n sentence "}]}]}"#).unwrap();
        assert_eq!(cues[0].start, 1.25);
        assert_eq!(cues[1].text, "next sentence");
        let vtt = vtt(&cues);
        assert!(vtt.contains("00:00:01.250 --> 00:00:03.750"));
        assert!(vtt.contains("&lt;script&gt; &amp; hello"));
        assert!(!vtt.contains("<script>"));
        assert!(parse_json3(br#"{"events":[]}"#).is_err());
    }
    #[test]
    fn caption_language_is_explicit_and_prefers_manual_without_changing_audio() {
        let sources:Sources=serde_json::from_value(serde_json::json!({"resolved_at":1,"original_language":"en-US","tracks":[{"url":"https://example.test/en","lang":"en","lang_name":"English","auto_generated":false},{"url":"https://example.test/ru","lang":"ru","lang_name":"Russian","auto_generated":false},{"url":"https://example.test/auto","lang":"en-US","lang_name":"English auto","auto_generated":true}]})).unwrap();
        assert_eq!(choose(&sources, "original", true).unwrap().lang, "en");
        assert_eq!(choose(&sources, "ru", false).unwrap().lang, "ru");
        assert!(choose(&sources, "ja", true).is_err());
    }
}

#[cfg(test)]
mod live_test {
    #[tokio::test]
    #[ignore = "Live optional YouTube caption check; downloads captions only"]
    async fn retrieves_available_youtube_captions_without_downloading_video() {
        use super::*;
        let url = std::env::var("ZIMSTUDIO_CAPTIONS_TEST_URL")
            .expect("Set ZIMSTUDIO_CAPTIONS_TEST_URL to a captioned YouTube watch URL");
        let temp = tempfile::tempdir().unwrap();
        tokio::fs::create_dir_all(temp.path().join("media"))
            .await
            .unwrap();
        let mut job:crate::Job=serde_json::from_value(serde_json::json!({"id":"caption-live","config":{"url":url,"title":"Caption transport check","kind":"media","max_items":1,"max_depth":0,"delay_ms":1000,"max_resource_mb":5,"language":"eng","description":"Caption transport check","creator":"Test","include_assets":true},"status":"downloading","phase":"download","created_at":1,"updated_at":1,"entries":[],"queue":[],"visited":[],"resources":{},"logs":[],"error":null,"output":null,"output_bytes":null,"retry_at":null,"requests":0,"bytes":0})).unwrap();
        let mut entry = crate::native_media::entry(
            "caption-test",
            "Caption transport check".into(),
            None,
            String::new(),
            false,
        );
        entry.url = url;
        entry.state = "done".into();
        job.entries = vec![entry];
        let record = Record {
            job: tokio::sync::Mutex::new(job),
            stop: Default::default(),
            running: Default::default(),
            import_cancel: Default::default(),
            dir: temp.path().to_owned(),
        };
        let rp = RustyPipe::builder()
            .storage_dir(temp.path().join("extractor"))
            .no_reporter()
            .timeout(Duration::from_secs(30))
            .n_http_retries(1)
            .build()
            .unwrap();
        prepare(&record, &rp).await.unwrap();
        let job = record.job.lock().await;
        assert!(
            !job.entries[0].subtitles.is_empty(),
            "{:?}",
            job.entries[0].subtitle_error
        );
        let file = job.entries[0].transcript_file.as_ref().unwrap();
        let cues: Vec<Cue> = serde_json::from_slice(
            &tokio::fs::read(temp.path().join("media").join(file))
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(!cues.is_empty());
        println!(
            "Caption transport succeeded: {} tracks and {} timed cues; no video downloaded",
            job.entries[0].subtitles.len(),
            cues.len()
        );
    }
}
