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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_start_small_jobs: true,
            auto_start_item_limit: 100,
            archive_template: ArchiveTemplate::Atlas,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> anyhow::Result<()> {
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
        super::web::http_url(&self.url)?;
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
pub struct Job {
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
    /// Incremented for each rebuild so exported revisions have distinct names.
    #[serde(default)]
    pub archive_revision: u64,
    pub created_at: u64,
    pub updated_at: u64,
    pub entries: Vec<Entry>,
    pub queue: Vec<PendingPage>,
    pub visited: Vec<String>,
    pub resources: BTreeMap<String, Resource>,
    pub logs: Vec<Log>,
    pub error: Option<String>,
    pub output: Option<String>,
    pub output_bytes: Option<u64>,
    pub retry_at: Option<u64>,
    pub requests: u64,
    pub bytes: u64,
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
            if r.state == "failed" {
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

    pub fn can_auto_start(&self) -> bool {
        self.auto_start_item_limit.is_some_and(|limit| {
            !self.scan_truncated
                && !self.entries.is_empty()
                && self.entries.len() <= limit
                && self.entries.iter().all(|entry| {
                    entry.selected && entry.state == "pending" && entry.error.is_none()
                })
        })
    }
    pub fn view(mut self) -> Self {
        self.queue.clear();
        self.visited.clear();
        self.resources.clear();
        for entry in &mut self.entries {
            entry.text.clear();
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
