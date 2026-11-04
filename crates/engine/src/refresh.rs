use crate::{web, Entry, Job, Record, SourceKind};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RefreshState {
    #[serde(default)]
    pub previous: Vec<Entry>,
    #[serde(default)]
    pub previous_quality: crate::capture::VideoQuality,
    #[serde(default)]
    pub fingerprints: BTreeMap<String, String>,
    pub compared: bool,
    pub complete: bool,
    pub changes: Vec<Change>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub url: String,
    pub title: String,
    pub kind: String,
}

pub async fn fingerprint(path: impl AsRef<std::path::Path>) -> Result<String> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub async fn compare(record: &Record) -> Result<()> {
    let snapshot = record.job.lock().await.clone();
    let Some(mut refresh) = snapshot.refresh.filter(|r| !r.compared) else {
        return Ok(());
    };
    let old: BTreeMap<_, _> = refresh
        .previous
        .iter()
        .map(|entry| (entry.url.as_str(), entry))
        .collect();
    let mut entries = snapshot.entries;
    let mut changes = Vec::new();
    let mut found = BTreeSet::new();
    for entry in &mut entries {
        found.insert(entry.url.clone());
        let previous = old.get(entry.url.as_str());
        let kind = if entry.state == "scan_failed" {
            "unavailable"
        } else if let Some(previous) = previous {
            entry.selected = previous.selected;
            let changed = if snapshot.config.kind == SourceKind::Website
                && entry.embedded_in.is_empty()
            {
                let hash = fingerprint(record.dir.join("cache").join(format!("{}.html", entry.id)))
                    .await?;
                refresh.fingerprints.get(&entry.id) != Some(&hash)
            } else {
                entry.title != previous.title || entry.duration != previous.duration
            };
            if (snapshot.config.kind == SourceKind::Media || !entry.embedded_in.is_empty())
                && previous.state == "done"
                && snapshot.config.capture.video_quality == refresh.previous_quality
                && web::http_url(&entry.url).is_ok_and(|url| crate::native_media::youtube(&url))
            {
                let mut files = previous
                    .media_file
                    .iter()
                    .chain(previous.media_audio.iter())
                    .chain(previous.thumbnail_file.iter());
                let mut valid = previous.media_file.is_some();
                for name in &mut files {
                    valid &= tokio::fs::metadata(record.dir.join("media").join(name))
                        .await
                        .is_ok_and(|m| m.is_file());
                }
                if valid {
                    entry.state = "done".into();
                    entry.media_file = previous.media_file.clone();
                    entry.media_audio = previous.media_audio.clone();
                    entry.thumbnail_file = previous.thumbnail_file.clone();
                    entry.size = previous.size;
                    entry.subtitles = previous.subtitles.clone();
                    entry.transcript = previous.transcript.clone();
                    entry.transcript_file = previous.transcript_file.clone();
                    entry.subtitle_policy = previous.subtitle_policy.clone();
                    entry.subtitle_error = previous.subtitle_error.clone();
                    if entry.text.is_empty() {
                        entry.text = previous.text.clone();
                    }
                }
            }
            if changed {
                "changed"
            } else {
                "unchanged"
            }
        } else {
            "new"
        };
        changes.push(Change {
            url: entry.url.clone(),
            title: entry.title.clone(),
            kind: kind.into(),
        });
    }
    for previous in &refresh.previous {
        if !found.contains(&previous.url) {
            changes.push(Change {
                url: previous.url.clone(),
                title: previous.title.clone(),
                kind: "not_seen".into(),
            });
        }
    }
    refresh.complete = !snapshot.scan_truncated && entries.iter().all(|e| e.state != "scan_failed");
    refresh.compared = true;
    refresh.changes = changes;
    refresh.previous.clear();
    refresh.fingerprints.clear();
    record.change(|job| {
        job.entries = entries;
        job.refresh = Some(refresh);
        job.log("info", "Refresh scan complete. Review new, changed, unchanged, and unseen items before building. Saved media is reused; existing ZIM copies remain unchanged.");
    }).await
}

impl crate::Engine {
    pub async fn refresh_collection(&self, id: &str) -> Result<Job> {
        use std::sync::atomic::Ordering;
        let record = self.record(id).await?;
        let mut current = record.job.lock().await;
        anyhow::ensure!(
            current.import.is_none(),
            "Imported collections can be rebuilt from saved files; start a new import to add files"
        );
        anyhow::ensure!(
            current.status == "completed" && !record.running.load(Ordering::SeqCst),
            "Finish the current archive before refreshing"
        );
        let mut next = current.clone();
        next.sitemap_checkpoint = Default::default();
        let mut refresh = RefreshState {
            previous: next.entries.clone(),
            previous_quality: next.config.capture.video_quality,
            ..Default::default()
        };
        if next.config.kind == SourceKind::Website {
            for entry in &next.entries {
                if let Ok(hash) =
                    fingerprint(record.dir.join("cache").join(format!("{}.html", entry.id))).await
                {
                    refresh.fingerprints.insert(entry.id.clone(), hash);
                }
            }
        }
        if next.config.kind == SourceKind::Media
            && !web::http_url(&next.config.url).is_ok_and(|u| crate::native_media::youtube(&u))
        {
            for entry in &next.entries {
                if let Some(file) = &entry.media_file {
                    let receipt = record
                        .dir
                        .join("media")
                        .join(file)
                        .with_extension("complete.json");
                    match tokio::fs::remove_file(receipt).await {
                        Ok(()) => (),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                        Err(e) => return Err(e.into()),
                    }
                }
            }
        }
        let history = record.dir.join("refresh-history");
        tokio::fs::create_dir_all(&history).await?;
        tokio::fs::copy(
            record.dir.join("job.json"),
            history.join(format!("{}.json", uuid::Uuid::new_v4())),
        )
        .await?;
        for file in ["media-cursor.json", "youtube-playlist-page.json"] {
            match tokio::fs::remove_file(record.dir.join(file)).await {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
        next.archive_revision = next
            .archive_revision
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("Archive revision limit reached"))?;
        next.downloaded_bytes = 0;
        next.refresh = Some(refresh);
        if next.config.kind == SourceKind::Website {
            for resource in next
                .resources
                .values_mut()
                .filter(|r| ["done", "failed", "skipped"].contains(&r.state.as_str()))
            {
                resource.state = "pending".into();
                resource.error = None;
            }
        }
        next.entries.clear();
        next.visited.clear();
        next.queue = next.config.starting_pages();
        next.scan_truncated = false;
        next.phase = "scan".into();
        next.status = "scanning".into();
        next.error = None;
        next.progress = Default::default();
        next.log("info", "Refreshing source metadata. Automatic approval is disabled for refreshes. Your existing ZIM stays intact until an updated archive succeeds.");
        record.save(&next).await?;
        *current = next;
        drop(current);
        self.launch(record)?;
        self.get(id).await
    }
}
