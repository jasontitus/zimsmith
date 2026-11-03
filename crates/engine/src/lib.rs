mod archive;
mod media;
pub mod model;
mod mux;
mod native_media;
pub mod templates;
mod thumbnails;
pub mod web;
pub mod youtube_audio;
mod youtube_playlist;

use anyhow::{Context, Result};
pub use model::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::{Mutex, Notify, RwLock};

pub struct Record {
    pub job: Mutex<Job>,
    pub stop: Notify,
    pub running: AtomicBool,
    pub dir: PathBuf,
}

impl Record {
    /// Frequent UI updates stay in memory; normal job checkpoints persist the
    /// latest snapshot without rewriting the entire manifest for each chunk.
    pub async fn progress(
        &self,
        label: impl Into<String>,
        completed: u64,
        total: Option<u64>,
        unit: &str,
    ) {
        let mut job = self.job.lock().await;
        job.progress = Progress {
            label: label.into(),
            completed,
            total,
            unit: unit.into(),
            updated_at: now(),
            ..Progress::default()
        };
    }
    pub async fn transfer_progress(&self, received: u64, expected: Option<u64>) {
        let mut job = self.job.lock().await;
        job.progress.received = received;
        job.progress.expected = expected;
        job.progress.updated_at = now();
    }
    pub async fn save(&self, job: &Job) -> Result<()> {
        // Write a fresh file and atomically replace the manifest, including on Windows.
        use std::io::Write;
        let mut file = atomic_write_file::AtomicWriteFile::open(self.dir.join("job.json"))?;
        file.write_all(&serde_json::to_vec(job)?)?;
        file.commit()?;
        Ok(())
    }
    pub async fn change(&self, update: impl FnOnce(&mut Job)) -> Result<()> {
        let mut job = self.job.lock().await;
        update(&mut job);
        job.updated_at = now();
        self.save(&job).await
    }
    pub async fn log(&self, level: &str, message: impl Into<String>) -> Result<()> {
        self.change(|j| j.log(level, message)).await
    }
}

#[derive(Clone)]
pub struct Engine {
    root: PathBuf,
    jobs: Arc<RwLock<HashMap<String, Arc<Record>>>>,
    settings: Arc<RwLock<Settings>>,
}

impl Engine {
    pub async fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        tokio::fs::create_dir_all(&root).await?;
        let settings = match tokio::fs::read(root.join("settings.json")).await {
            Ok(data) => {
                serde_json::from_slice::<Settings>(&data).context("Could not read app settings")?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(error) => return Err(error.into()),
        };
        settings.validate()?;
        let engine = Self {
            root: root.clone(),
            jobs: Default::default(),
            settings: Arc::new(RwLock::new(settings)),
        };
        let mut dirs = tokio::fs::read_dir(&root).await?;
        while let Some(dir) = dirs.next_entry().await? {
            if !dir.file_type().await?.is_dir() {
                continue;
            }
            let path = dir.path().join("job.json");
            if !path.exists() {
                continue;
            }
            let data = tokio::fs::read(&path).await?;
            let mut job: Job = serde_json::from_slice(&data)
                .with_context(|| format!("Could not recover {}", path.display()))?;
            if ["scanning", "downloading", "packaging", "pausing"].contains(&job.status.as_str()) {
                job.status = "paused".into();
                job.log(
                    "warning",
                    "Recovered after shutdown. Resume from the last checkpoint.",
                );
            }
            let record = Arc::new(Record {
                job: Mutex::new(job.clone()),
                stop: Notify::new(),
                running: AtomicBool::new(false),
                dir: dir.path(),
            });
            record.save(&job).await?;
            engine.jobs.write().await.insert(job.id, record);
        }
        Ok(engine)
    }
    pub async fn list(&self) -> Vec<Job> {
        let records: Vec<_> = self.jobs.read().await.values().cloned().collect();
        let mut jobs = Vec::new();
        for record in records {
            jobs.push(record.job.lock().await.clone());
        }
        jobs.sort_by_key(|j| std::cmp::Reverse(j.created_at));
        jobs
    }
    pub async fn record(&self, id: &str) -> Result<Arc<Record>> {
        self.jobs
            .read()
            .await
            .get(id)
            .cloned()
            .context("Job not found")
    }
    pub async fn get(&self, id: &str) -> Result<Job> {
        Ok(self.record(id).await?.job.lock().await.clone())
    }
    pub async fn settings(&self) -> Settings {
        self.settings.read().await.clone()
    }

    pub async fn update_settings(&self, settings: Settings) -> Result<Settings> {
        settings.validate()?;
        let mut current = self.settings.write().await;
        use std::io::Write;
        let mut file = atomic_write_file::AtomicWriteFile::open(self.root.join("settings.json"))?;
        file.write_all(&serde_json::to_vec(&settings)?)?;
        file.commit()?;
        *current = settings.clone();
        Ok(settings)
    }
    pub async fn create(&self, mut config: NewJob) -> Result<Job> {
        config.url = web::source_url(&config.url)?.to_string();
        let detected_media = config.detect_media_source();
        config.validate()?;
        let settings = self.settings().await;
        let id = uuid::Uuid::new_v4().to_string();
        let mut job = Job {
            id: id.clone(),
            queue: vec![PendingPage {
                url: config.url.clone(),
                depth: 0,
            }],
            config,
            status: "scanning".into(),
            phase: "scan".into(),
            auto_start_item_limit: settings
                .auto_start_small_jobs
                .then_some(settings.auto_start_item_limit),
            scan_truncated: false,
            archive_template: settings.archive_template,
            archive_revision: 0,
            created_at: now(),
            updated_at: now(),
            entries: vec![],
            visited: vec![],
            resources: Default::default(),
            logs: vec![],
            error: None,
            output: None,
            output_bytes: None,
            retry_at: None,
            requests: 0,
            bytes: 0,
            progress: Progress::default(),
        };
        if detected_media {
            job.log(
                "info",
                "Media URL detected. Using media collection scanning.",
            );
        }
        let dir = self.root.join(&id);
        tokio::fs::create_dir_all(dir.join("cache")).await?;
        let record = Arc::new(Record {
            job: Mutex::new(job.clone()),
            stop: Notify::new(),
            running: AtomicBool::new(false),
            dir,
        });
        record.save(&job).await?;
        self.jobs.write().await.insert(id, record.clone());
        self.launch(record)?;
        Ok(job)
    }
    fn launch(&self, record: Arc<Record>) -> Result<()> {
        anyhow::ensure!(
            !record.running.swap(true, Ordering::SeqCst),
            "Job is still pausing; try again shortly"
        );
        tokio::spawn(async move {
            let stop = record.stop.notified();
            tokio::pin!(stop);
            stop.as_mut().enable();
            if record.job.lock().await.status == "pausing" {
                let _ = record.change(|j| j.status = "paused".into()).await;
                record.running.store(false, Ordering::SeqCst);
                return;
            }
            let result = tokio::select! {
                biased;
                _ = &mut stop => Ok(()),
                result = drive(&record) => result,
            };
            if let Err(error) = result {
                let message = format!("{error:#}");
                let _ = record
                    .change(|j| {
                        j.status = "failed".into();
                        j.error = Some(message.clone());
                        j.log("error", message);
                    })
                    .await;
            }
            let _ = record
                .change(|j| {
                    if j.status == "pausing" {
                        j.status = "paused".into();
                        j.log(
                            "info",
                            "Paused. Checkpoints and partial media downloads are safe.",
                        );
                    }
                })
                .await;
            record.running.store(false, Ordering::SeqCst);
        });
        Ok(())
    }
    pub async fn pause(&self, id: &str) -> Result<Job> {
        let record = self.record(id).await?;
        {
            let mut job = record.job.lock().await;
            anyhow::ensure!(
                ["scanning", "downloading", "packaging"].contains(&job.status.as_str()),
                "This job is not running"
            );
            job.status = "pausing".into();
            record.save(&job).await?;
            record.stop.notify_waiters();
        }
        self.get(id).await
    }
    pub async fn resume(&self, id: &str) -> Result<Job> {
        let record = self.record(id).await?;
        anyhow::ensure!(
            !record.running.load(Ordering::SeqCst),
            "Job is still stopping; try again shortly"
        );
        {
            let mut job = record.job.lock().await;
            anyhow::ensure!(
                ["paused", "failed"].contains(&job.status.as_str()),
                "Only paused or failed jobs can resume"
            );
            job.prepare_resume();
            record.save(&job).await?;
        }
        self.launch(record)?;
        self.get(id).await
    }
    pub async fn edit_retry(&self, id: &str, mut config: NewJob) -> Result<Job> {
        config.url = web::source_url(&config.url)?.to_string();
        config.detect_media_source();
        config.validate()?;
        let record = self.record(id).await?;
        anyhow::ensure!(
            !record.running.load(Ordering::SeqCst),
            "Job is still stopping; try again shortly"
        );
        {
            let mut current = record.job.lock().await;
            anyhow::ensure!(
                ["failed", "paused"].contains(&current.status.as_str()),
                "Pause the crawl or wait for it to fail before editing"
            );
            let source_changed =
                config.url != current.config.url || config.kind != current.config.kind;
            let rescan = source_changed
                || config.max_items != current.config.max_items
                || config.max_depth != current.config.max_depth
                || config.include_assets != current.config.include_assets;
            let mut job = current.clone();
            job.config = config;
            if job.output.is_some() {
                job.archive_revision = job
                    .archive_revision
                    .checked_add(1)
                    .context("Archive revision limit reached")?;
            }
            // Keep the old manifest and all downloaded payloads for recovery.
            let history = record.dir.join("edit-history");
            tokio::fs::create_dir_all(&history).await?;
            tokio::fs::copy(
                record.dir.join("job.json"),
                history.join(format!("{}.json", uuid::Uuid::new_v4())),
            )
            .await?;
            if rescan {
                for file in ["media-cursor.json", "youtube-playlist-page.json"] {
                    match tokio::fs::remove_file(record.dir.join(file)).await {
                        Ok(()) => (),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                        Err(error) => return Err(error.into()),
                    }
                }
                job.entries.clear();
                job.visited.clear();
                job.queue = vec![PendingPage {
                    url: job.config.url.clone(),
                    depth: 0,
                }];
                job.scan_truncated = false;
                job.phase = "scan".into();
                job.progress = Progress::default();
                if source_changed {
                    job.resources.clear();
                    job.bytes = 0;
                }
                job.log("info", "Crawl source or scan limits changed. Restarting discovery; saved files are kept and reused where applicable.");
            } else {
                job.log(
                    "info",
                    "Crawl settings updated. Keeping saved progress and downloaded files.",
                );
            }
            job.prepare_resume();
            record.save(&job).await?;
            *current = job;
        }
        self.launch(record)?;
        self.get(id).await
    }
    pub async fn build(&self, id: &str, selected: Vec<String>) -> Result<Job> {
        let record = self.record(id).await?;
        anyhow::ensure!(
            !record.running.load(Ordering::SeqCst),
            "Job is still running"
        );
        {
            let mut job = record.job.lock().await;
            anyhow::ensure!(
                ["review", "failed"].contains(&job.status.as_str()),
                "Finish scanning before confirming the download"
            );
            anyhow::ensure!(!selected.is_empty(), "Select at least one item");
            anyhow::ensure!(
                selected.iter().all(|id| job
                    .entries
                    .iter()
                    .any(|e| &e.id == id && e.state != "scan_failed")),
                "Selection contains an unavailable item"
            );
            for entry in &mut job.entries {
                entry.selected = selected.contains(&entry.id);
                if entry.state == "failed" {
                    entry.state = "pending".into();
                    entry.error = None;
                }
            }
            for r in job.resources.values_mut() {
                if r.state == "failed" {
                    r.state = "pending".into();
                    r.error = None;
                }
            }
            job.phase = "download".into();
            job.status = "downloading".into();
            job.error = None;
            job.log(
                "info",
                format!("Confirmed {} items. Beginning download.", selected.len()),
            );
            record.save(&job).await?;
        }
        self.launch(record)?;
        self.get(id).await
    }
    /// Re-resolve YouTube audio without rescanning or changing the approved selection.
    pub async fn repair_youtube_audio(&self, id: &str) -> Result<Job> {
        let record = self.record(id).await?;
        anyhow::ensure!(
            !record.running.load(Ordering::SeqCst),
            "Pause the running job and wait for it to stop before repairing audio"
        );
        {
            let mut current = record.job.lock().await;
            let mut next = current.clone();
            next.prepare_original_audio_repair()?;
            let history = record.dir.join("audio-repair-history");
            tokio::fs::create_dir_all(&history).await?;
            tokio::fs::copy(
                record.dir.join("job.json"),
                history.join(format!("{}.json", uuid::Uuid::new_v4())),
            )
            .await?;
            record.save(&next).await?;
            *current = next;
        }
        self.launch(record)?;
        self.get(id).await
    }

    /// Repackage only: never send completed content through scan/download again.
    pub async fn rebuild(&self, id: &str, template: ArchiveTemplate) -> Result<Job> {
        let record = self.record(id).await?;
        anyhow::ensure!(
            !record.running.load(Ordering::SeqCst),
            "Job is still running; try again shortly"
        );
        {
            let mut job = record.job.lock().await;
            anyhow::ensure!(
                job.phase == "package"
                    && ["completed", "failed", "paused"].contains(&job.status.as_str()),
                "Finish downloading this collection before rebuilding"
            );
            let selected: Vec<_> = job.entries.iter().filter(|e| e.selected).collect();
            anyhow::ensure!(
                !selected.is_empty() && selected.iter().all(|e| e.state == "done"),
                "All selected items must be downloaded before rebuilding"
            );
            for entry in selected {
                let files = match job.config.kind {
                    SourceKind::Website => {
                        vec![record.dir.join("cache").join(format!("{}.html", entry.id))]
                    }
                    SourceKind::Media => {
                        let file = entry
                            .media_file
                            .as_ref()
                            .context("Saved media file checkpoint is missing")?;
                        let mut files = vec![record.dir.join("media").join(file)];
                        if let Some(audio) = &entry.media_audio {
                            files.push(record.dir.join("media").join(audio));
                        }
                        files
                    }
                };
                for file in files {
                    anyhow::ensure!(tokio::fs::metadata(file).await.is_ok_and(|m| m.is_file()), "The saved content for {} is missing. Restore the downloaded files before rebuilding.", entry.title);
                }
            }
            job.archive_template = template;
            job.archive_revision = job
                .archive_revision
                .checked_add(1)
                .context("Archive revision limit reached")?;
            job.status = "packaging".into();
            job.error = None;
            job.progress = Progress {
                label: "Rebuilding from downloaded files".into(),
                updated_at: now(),
                ..Progress::default()
            };
            job.log("info", format!("Rebuilding with {} from saved files. Pages and media will not be downloaded again.", template.id()));
            record.save(&job).await?;
        }
        self.launch(record)?;
        self.get(id).await
    }
    pub async fn artifact(&self, id: &str) -> Result<PathBuf> {
        let record = self.record(id).await?;
        let job = record.job.lock().await;
        anyhow::ensure!(job.status == "completed", "Archive is not ready");
        Ok(record.dir.join("archive.zim"))
    }
    pub async fn capabilities(&self) -> Capabilities {
        let yt_dlp = media::probe("yt-dlp", "--version").await;
        let zimwriterfs = ToolStatus {
            available: true,
            detail: "Embedded zimru Rust writer".into(),
        };
        Capabilities {
            version: env!("CARGO_PKG_VERSION").into(),
            platform: std::env::consts::OS.into(),
            local_execution: true,
            yt_dlp,
            zimwriterfs,
            data_dir: self.root.display().to_string(),
        }
    }
}

async fn drive(record: &Record) -> Result<()> {
    let job = record.job.lock().await.clone();
    if job.phase == "scan" {
        record
            .progress("Scanning source metadata", 0, None, "items")
            .await;
        record.log("info", "Scanning metadata only. Website HTML is cached; media and page assets wait until scanning finishes and the download is approved.").await?;
        match job.config.kind {
            SourceKind::Website => web::scan(record).await?,
            SourceKind::Media => media::scan(record).await?,
        }
        record
            .change(|j| {
                if j.can_auto_start() {
                    j.phase = "download".into();
                    j.status = "downloading".into();
                    j.log("info", format!("Automatically approved {} items using the saved threshold of {}. Starting download and ZIM build.", j.entries.len(), j.auto_start_item_limit.unwrap()));
                } else {
                    j.status = "review".into();
                    j.log("info", format!("Scan complete: {} items. Review and confirm your selection.", j.entries.len()));
                }
                j.progress.completed = j.entries.len() as u64;
                j.progress.total = Some(j.entries.len() as u64);
                j.progress.label = "Scan complete".into();
                j.progress.updated_at = now();
            })
            .await?;
    }
    let (phase, status) = {
        let job = record.job.lock().await;
        (job.phase.clone(), job.status.clone())
    };
    if status == "review" {
        return Ok(());
    }
    if phase != "scan" {
        if phase == "download" {
            match job.config.kind {
                SourceKind::Website => web::download(record).await?,
                SourceKind::Media => media::download(record).await?,
            }
            record
                .change(|j| {
                    j.phase = "package".into();
                    j.status = "packaging".into();
                    j.log(
                        "info",
                        "Building table of contents and full-text search index.",
                    );
                })
                .await?;
        }
        archive::package(record).await?;
    }
    Ok(())
}
