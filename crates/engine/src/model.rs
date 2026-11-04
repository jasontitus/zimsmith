use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveTemplate {
    #[default]
    Atlas,
    FieldNotes,
    Midnight,
    ScreeningRoom,
}

impl ArchiveTemplate {
    pub fn id(self) -> &'static str {
        match self {
            Self::Atlas => "atlas",
            Self::FieldNotes => "field_notes",
            Self::Midnight => "midnight",
            Self::ScreeningRoom => "screening_room",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub auto_start_small_jobs: bool,
    pub auto_start_item_limit: usize,
    pub archive_template: ArchiveTemplate,
    pub zstd_level: i32,
}

pub fn default_zstd_level() -> i32 {
    3
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_start_small_jobs: true,
            auto_start_item_limit: 100,
            archive_template: ArchiveTemplate::Atlas,
            zstd_level: default_zstd_level(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=22).contains(&self.zstd_level),
            "Zstandard compression level must be 1–22"
        );
        anyhow::ensure!(
            (1..=5000).contains(&self.auto_start_item_limit),
            "Automatic download threshold must be 1–5000 items"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Website,
    Media,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewJob {
    pub url: String,
    pub title: String,
    pub kind: SourceKind,
    pub max_items: usize,
    pub max_depth: usize,
    pub delay_ms: u64,
    pub max_resource_mb: u64,
    pub language: String,
    pub description: String,
    pub creator: String,
    pub include_assets: bool,
    #[serde(default)]
    pub capture: crate::capture::CaptureOptions,
}

impl NewJob {
    /// Known media URLs must reach an extractor, including clients with an old
    /// default of Website. Unknown providers can still be selected as Media.
    pub fn detect_media_source(&mut self) -> bool {
        if self.kind == SourceKind::Website && super::native_media::supports(&self.url) {
            self.kind = SourceKind::Media;
            true
        } else {
            false
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        self.capture.validate(&super::web::http_url(&self.url)?)?;
        anyhow::ensure!(
            !self.capture.render_javascript || self.kind == SourceKind::Website,
            "Browser rendering applies to website crawls"
        );
        anyhow::ensure!(
            self.kind == SourceKind::Website || self.capture.seed_urls.is_empty(),
            "URL lists support websites; use a channel or playlist for media collections"
        );
        anyhow::ensure!(
            !self.title.trim().is_empty() && self.title.len() <= 120,
            "Title must contain 1–120 characters"
        );
        anyhow::ensure!(
            (1..=5000).contains(&self.max_items),
            "Item limit must be 1–5000"
        );
        anyhow::ensure!(self.max_depth <= 20, "Maximum depth is 20");
        anyhow::ensure!(
            (1000..=60000).contains(&self.delay_ms),
            "Request spacing must be 1–60 seconds"
        );
        anyhow::ensure!(
            (1..=1024).contains(&self.max_resource_mb),
            "Resource limit must be 1–1024 MB"
        );
        anyhow::ensure!(
            self.language.len() == 3 && self.language.bytes().all(|b| b.is_ascii_lowercase()),
            "Use a three-letter language code, such as eng"
        );
        anyhow::ensure!(!self.creator.trim().is_empty(), "Creator is required");
        anyhow::ensure!(
            !self.description.trim().is_empty() && self.description.len() <= 80,
            "Description must contain 1–80 characters"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub rendered: Option<crate::render::RenderInfo>,
    /// Website pages which reference this deduplicated, separately approved video.
    #[serde(default)]
    pub embedded_in: Vec<String>,
    pub id: String,
    pub url: String,
    pub title: String,
    pub depth: usize,
    pub selected: bool,
    pub state: String,
    pub size: Option<u64>,
    pub duration: Option<f64>,
    pub error: Option<String>,
    pub text: String,
    pub assets: Vec<String>,
    pub media_file: Option<String>,
    #[serde(default)]
    pub subtitles: Vec<crate::captions::SavedSubtitle>,
    #[serde(default)]
    pub transcript: Vec<crate::captions::Cue>,
    #[serde(default)]
    pub transcript_file: Option<String>,
    #[serde(default)]
    pub subtitle_policy: Option<String>,
    #[serde(default)]
    pub subtitle_error: Option<String>,
    #[serde(default)]
    pub media_audio: Option<String>,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub thumbnail_file: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingPage {
    pub url: String,
    pub depth: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resource {
    pub url: String,
    pub path: String,
    pub state: String,
    pub error: Option<String>,
    pub mime: String,
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Log {
    pub time: u64,
    pub level: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Progress {
    pub label: String,
    pub completed: u64,
    pub total: Option<u64>,
    pub unit: String,
    pub received: u64,
    pub expected: Option<u64>,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssetSummary {
    pub downloaded: usize,
    pub missing: usize,
    pub omitted: usize,
    #[serde(default)]
    pub failed: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    #[serde(default)]
    pub queued_pages: usize,
    #[serde(default)]
    pub javascript_needed: bool,
    #[serde(default)]
    pub sitemap_checkpoint: crate::sitemap::SitemapState,
    pub id: String,
    pub config: NewJob,
    pub status: String,
    pub phase: String,
    // Snapshot the preference when a job is created. Old manifests keep manual review.
    #[serde(default)]
    pub auto_start_item_limit: Option<usize>,
    #[serde(default)]
    pub scan_truncated: bool,
    #[serde(default)]
    pub archive_template: ArchiveTemplate,
    /// Snapshot for new jobs; explicit rebuilds use the current preference.
    #[serde(default = "default_zstd_level")]
    pub zstd_level: i32,
    /// Incremented for each rebuild so exported revisions have distinct names.
    #[serde(default)]
    pub archive_revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
    pub entries: Vec<Entry>,
    pub queue: Vec<PendingPage>,
    pub visited: Vec<String>,
    pub resources: BTreeMap<String, Resource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_summary: Option<AssetSummary>,
    #[serde(default)]
    pub refresh: Option<crate::refresh::RefreshState>,
    #[serde(default)]
    pub import: Option<crate::imports::ImportState>,
    #[serde(default)]
    pub capture_report: Option<crate::capture_report::Summary>,
    #[serde(default)]
    pub validation: Option<crate::capture_report::Validation>,
    pub logs: Vec<Log>,
    pub error: Option<String>,
    pub output: Option<String>,
    pub output_bytes: Option<u64>,
    pub retry_at: Option<u64>,
    pub requests: u64,
    pub bytes: u64,
    #[serde(default)]
    pub downloaded_bytes: u64,
    #[serde(default)]
    pub progress: Progress,
}

impl Job {
    pub(crate) fn prepare_resume(&mut self) {
        self.status = match self.phase.as_str() {
            "scan" => "scanning",
            "package" => "packaging",
            _ => "downloading",
        }
        .into();
        self.error = None;
        self.correct_scan_source();
        if self.phase == "scan" && matches!(self.config.kind, SourceKind::Website) {
            let failed: Vec<_> = self
                .entries
                .iter()
                .filter(|e| e.state == "scan_failed")
                .map(|e| PendingPage {
                    url: e.url.clone(),
                    depth: e.depth,
                })
                .collect();
            for page in &failed {
                self.visited.retain(|url| url != &page.url);
            }
            self.queue.extend(failed);
            self.entries.retain(|e| e.state != "scan_failed");
        }
        self.log("info", "Resuming from checkpoint.");
        for e in &mut self.entries {
            if e.state == "failed" {
                e.state = "pending".into();
                e.error = None;
            }
        }
        for r in self.resources.values_mut() {
            if r.state == "failed" || r.state == "excluded" {
                r.state = "pending".into();
                r.error = None;
            }
        }
    }

    pub fn correct_scan_source(&mut self) {
        if self.phase == "scan" && self.config.detect_media_source() {
            // Old HTML scan checkpoints cannot be reused by a media extractor.
            self.entries.clear();
            self.queue.clear();
            self.visited.clear();
            self.resources.clear();
            self.scan_truncated = false;
            self.retry_at = None;
            self.log(
                "info",
                "Media URL detected. Restarting the scan with the on-device media extractor.",
            );
        }
    }

    pub(crate) fn prepare_original_audio_repair(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.config.kind == SourceKind::Media
                && super::native_media::youtube(&super::web::http_url(&self.config.url)?),
            "Original-audio repair is only available for YouTube collections"
        );
        anyhow::ensure!(
            ["completed", "failed", "paused"].contains(&self.status.as_str())
                && ["download", "package"].contains(&self.phase.as_str()),
            "Finish the scan and pause any active job before repairing audio"
        );
        let selected = self.entries.iter().filter(|e| e.selected).count();
        anyhow::ensure!(
            selected > 0
                && self
                    .entries
                    .iter()
                    .filter(|e| e.selected)
                    .all(|e| e.state != "scan_failed"),
            "Select available videos before repairing audio"
        );
        if self.output.is_some() {
            self.archive_revision = self
                .archive_revision
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("Archive revision limit reached"))?;
        }
        for entry in self.entries.iter_mut().filter(|e| e.selected) {
            entry.state = "pending".into();
            entry.error = None;
        }
        self.phase = "download".into();
        self.status = "downloading".into();
        self.error = None;
        self.progress = Progress {
            label: "Repairing original YouTube audio".into(),
            total: Some(selected as u64),
            unit: "items".into(),
            updated_at: now(),
            ..Progress::default()
        };
        self.log("info", "Re-resolving original audio for the selected videos. Matching video downloads are reused; audio may be downloaded again. The existing ZIM stays intact until a replacement is built.");
        Ok(())
    }

    pub fn can_auto_start(&self) -> bool {
        self.refresh.is_none()
            && !self.javascript_needed
            && !self.config.capture.render_javascript
            && self
                .entries
                .iter()
                .all(|e| e.embedded_in.is_empty() && e.rendered.is_none())
            && self.auto_start_item_limit.is_some_and(|limit| {
                !self.scan_truncated
                    && !self.entries.is_empty()
                    && self.entries.len() <= limit
                    && self.entries.iter().all(|entry| {
                        entry.selected && entry.state == "pending" && entry.error.is_none()
                    })
            })
    }
    pub fn view(mut self) -> Self {
        self.asset_summary = Some(AssetSummary {
            failed: self
                .resources
                .values()
                .filter(|resource| resource.state == "failed")
                .count(),
            downloaded: self
                .resources
                .values()
                .filter(|r| r.state == "done")
                .count(),
            missing: self
                .resources
                .values()
                .filter(|r| r.state != "done" && r.state != "skipped")
                .count(),
            omitted: self
                .resources
                .values()
                .filter(|r| r.state == "skipped")
                .count(),
        });
        if let Some(import) = &mut self.import {
            import.file_count = import.file_count.max(import.files.len());
            import.files.clear();
        }
        if let Some(refresh) = &mut self.refresh {
            refresh.previous.clear();
            refresh.fingerprints.clear();
        }
        self.queued_pages = self.queue.len();
        self.queue.clear();
        self.visited.clear();
        self.resources.clear();
        for entry in &mut self.entries {
            entry.text.clear();
            entry.transcript.clear();
            entry.assets.clear();
        }
        self
    }
    pub fn log(&mut self, level: &str, message: impl Into<String>) {
        self.logs.push(Log {
            time: now(),
            level: level.into(),
            message: message.into(),
        });
        if self.logs.len() > 250 {
            self.logs.remove(0);
        }
        self.updated_at = now();
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolStatus {
    pub available: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capabilities {
    pub version: String,
    pub platform: String,
    pub local_execution: bool,
    pub yt_dlp: ToolStatus,
    pub zimwriterfs: ToolStatus,
    pub data_dir: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_preferences_and_job_manifests_keep_level_three() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.zstd_level, 3);
        assert_eq!(completed().zstd_level, 3);
    }
    #[test]
    fn compression_preferences_accept_supported_levels_and_reject_invalid_values() {
        for zstd_level in [1, 3, 9, 22] {
            assert!(Settings {
                zstd_level,
                ..Default::default()
            }
            .validate()
            .is_ok());
        }
        for zstd_level in [-1, 0, 23] {
            assert!(Settings {
                zstd_level,
                ..Default::default()
            }
            .validate()
            .is_err());
        }
    }
    fn completed() -> Job {
        serde_json::from_value(serde_json::json!({
            "id":"repair-fixture", "config":{"url":"https://www.youtube.com/playlist?list=fixture","title":"Fixture","kind":"media","max_items":10,"max_depth":0,"delay_ms":1000,"max_resource_mb":50,"language":"eng","description":"Fixture","creator":"Fixture","include_assets":false},
            "status":"completed","phase":"package","created_at":1,"updated_at":1,"archive_revision":3,
            "entries":[{"id":"one","url":"https://www.youtube.com/watch?v=fixture1234","title":"One","depth":0,"selected":true,"state":"done","size":20,"duration":1,"error":null,"text":"","assets":[],"media_file":"one.mp4","media_audio":"one-audio.m4a"},
            {"id":"two","url":"https://www.youtube.com/watch?v=fixture5678","title":"Two","depth":0,"selected":false,"state":"done","size":20,"duration":1,"error":null,"text":"","assets":[],"media_file":"two.mp4","media_audio":null}],
            "queue":[],"visited":[],"resources":{},"logs":[],"error":null,"output":"archive.zim","output_bytes":42,"retry_at":9999999999u64,"requests":10,"bytes":40
        })).unwrap()
    }
    #[test]
    fn repairs_only_approved_items_and_preserves_video_archive_and_cooldown() {
        let mut job = completed();
        let source = job.config.url.clone();
        job.prepare_original_audio_repair().unwrap();
        assert_eq!(job.phase, "download");
        assert_eq!(job.status, "downloading");
        assert_eq!(job.archive_revision, 4);
        assert_eq!(job.config.url, source);
        assert_eq!(job.retry_at, Some(9999999999));
        assert_eq!(job.output.as_deref(), Some("archive.zim"));
        assert_eq!(job.entries[0].state, "pending");
        assert_eq!(job.entries[0].media_file.as_deref(), Some("one.mp4"));
        assert_eq!(job.entries[0].media_audio.as_deref(), Some("one-audio.m4a"));
        assert!(!job.entries[1].selected);
        assert_eq!(job.entries[1].state, "done");
    }
    #[test]
    fn rejects_active_unscanned_non_youtube_and_unavailable_selections_without_changes() {
        for (status, phase, url) in [
            (
                "downloading",
                "download",
                "https://www.youtube.com/watch?v=fixture1234",
            ),
            (
                "paused",
                "scan",
                "https://www.youtube.com/watch?v=fixture1234",
            ),
            ("completed", "package", "https://media.example/clip.mp4"),
        ] {
            let mut job = completed();
            job.status = status.into();
            job.phase = phase.into();
            job.config.url = url.into();
            let before = serde_json::to_vec(&job).unwrap();
            assert!(job.prepare_original_audio_repair().is_err());
            assert_eq!(serde_json::to_vec(&job).unwrap(), before);
        }
        let mut job = completed();
        job.entries[0].state = "scan_failed".into();
        assert!(job.prepare_original_audio_repair().is_err());
    }
}
