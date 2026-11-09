//! Local imports never access the network. Payloads use generated disk names;
//! user/ZIP paths are logical paths only, with traversal and size checks.
use crate::{web, Entry, Job, NewJob, Resource, SourceKind};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use tokio::io::AsyncWriteExt;
pub(crate) const MAX_FILES: usize = 50_000;
const MAX_TOTAL: u64 = 20 * 1024 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportFile {
    pub path: String,
    pub size: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ImportState {
    pub files: Vec<ImportFile>,
    pub finished: bool,
    #[serde(default)]
    pub file_count: usize,
    #[serde(default)]
    pub whole_warc: bool,
}
#[derive(Clone)]
pub(crate) struct Payload {
    pub path: String,
    pub file: std::path::PathBuf,
    pub size: u64,
    pub url: Option<String>,
    pub mime: Option<String>,
}

pub fn logical_path(value: &str) -> Result<String> {
    anyhow::ensure!(
        !value.is_empty()
            && value.len() <= 1024
            && !value.starts_with('/')
            && !value.contains(['\\', ':', '\0'])
            && !value.chars().any(char::is_control),
        "Unsupported or unsafe import path"
    );
    let mut parts = Vec::new();
    for part in value.split('/') {
        anyhow::ensure!(
            part != "..",
            "Import paths must not contain parent traversal"
        );
        if part != "." && !part.is_empty() {
            parts.push(part);
        }
    }
    anyhow::ensure!(!parts.is_empty(), "Empty import path");
    Ok(parts.join("/"))
}
fn local_url(path: &str) -> Result<url::Url> {
    let mut url = web::http_url("https://local-import.invalid/")?;
    url.path_segments_mut()
        .unwrap()
        .clear()
        .extend(path.split('/'));
    Ok(url)
}
fn extension(path: &str) -> String {
    path.rsplit('.').next().unwrap_or("").to_ascii_lowercase()
}
fn validate_files(files: &[ImportFile], config: &NewJob) -> Result<()> {
    anyhow::ensure!(
        !files.is_empty() && files.len() <= MAX_FILES,
        "Choose 1–50000 files"
    );
    let mut names = BTreeSet::new();
    let mut size = 0u64;
    for file in files {
        let name = logical_path(&file.path)?;
        anyhow::ensure!(
            names.insert(name.to_lowercase()),
            "Duplicate import filename: {name}"
        );
        anyhow::ensure!(
            file.size
                <= if crate::warc_import::recognized(&file.path) {
                    MAX_TOTAL
                } else {
                    config.max_resource_mb * 1024 * 1024
                },
            "Import file exceeds the per-file limit: {name}"
        );
        size = size
            .checked_add(file.size)
            .context("Import size overflow")?;
    }
    let cap = config
        .capture
        .max_download_mb
        .map(|mb| mb * 1024 * 1024)
        .unwrap_or(MAX_TOTAL)
        .min(MAX_TOTAL);
    anyhow::ensure!(
        size <= cap,
        "Import exceeds its total budget (maximum 20 GB); use fewer files or increase the budget"
    );
    Ok(())
}
fn expand_zip(
    file: &Path,
    output: &Path,
    cap: u64,
    file_cap: u64,
    control: std::sync::Arc<crate::Record>,
) -> Result<Vec<Payload>> {
    use std::io::{Read, Write};
    let mut archive = zip::ZipArchive::new(std::fs::File::open(file)?)?;
    anyhow::ensure!(archive.len() <= MAX_FILES, "ZIP contains too many entries");
    let mut result = Vec::new();
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    std::fs::create_dir_all(output)?;
    for i in 0..archive.len() {
        anyhow::ensure!(
            !control
                .import_cancel
                .load(std::sync::atomic::Ordering::SeqCst),
            "Import interrupted; reselect saved files to resume"
        );
        if let Ok(mut job) = control.job.try_lock() {
            job.progress.unit = "files".into();
            job.progress.received = 0;
            job.progress.expected = None;
            job.progress.completed = i as u64;
            job.progress.total = Some(archive.len() as u64);
            job.progress.updated_at = crate::now();
        }
        let mut item = archive.by_index(i)?;
        anyhow::ensure!(
            !item.is_symlink() && !item.encrypted(),
            "Encrypted ZIPs and symbolic links are not supported"
        );
        if item.is_dir() {
            continue;
        }
        anyhow::ensure!(
            item.enclosed_name().is_some(),
            "ZIP contains an unsafe path"
        );
        let name = logical_path(&item.name()?)?;
        anyhow::ensure!(
            names.insert(name.to_lowercase()),
            "ZIP contains duplicate paths"
        );
        anyhow::ensure!(
            item.size() <= file_cap,
            "ZIP entry exceeds the per-file limit: {name}"
        );
        total = total
            .checked_add(item.size())
            .context("ZIP size overflow")?;
        anyhow::ensure!(total <= cap, "Expanded ZIP exceeds the import budget");
        let path = output.join(web::key(&name));
        // Flat generated filenames prevent case-folding or path aliases from
        // escaping the staging directory, even on mobile filesystems.
        let mut target = std::fs::File::create(&path)?;
        let mut copied = 0u64;
        let declared = item.size();
        let mut buffer = [0; 64 * 1024];
        loop {
            anyhow::ensure!(
                !control
                    .import_cancel
                    .load(std::sync::atomic::Ordering::SeqCst),
                "Import interrupted; reselect saved files to resume"
            );
            let n = item.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            copied += n as u64;
            anyhow::ensure!(
                copied <= declared && copied <= file_cap,
                "ZIP entry expanded beyond its declared limit"
            );
            target.write_all(&buffer[..n])?;
        }
        anyhow::ensure!(copied == declared, "ZIP entry ended early");
        target.sync_all()?;
        result.push(Payload {
            path: name,
            file: path,
            size: copied,
            url: None,
            mime: None,
        });
    }
    Ok(result)
}
impl crate::Engine {
    pub async fn begin_import(
        &self,
        mut config: NewJob,
        mut files: Vec<ImportFile>,
        existing_id: Option<String>,
    ) -> Result<Job> {
        config.url = "https://local-import.invalid/".into();
        config.kind = SourceKind::Website;
        config.capture.seed_urls.clear();
        config.validate()?;
        for file in &mut files {
            file.path = logical_path(&file.path)?;
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        validate_files(&files, &config)?;
        if let Some(id) = existing_id {
            let record = self.record(&id).await?;
            let mut job = record.job.lock().await;
            anyhow::ensure!(
                !record.running.load(std::sync::atomic::Ordering::SeqCst),
                "Import files are already being prepared"
            );
            anyhow::ensure!(
                ["paused", "failed", "importing"].contains(&job.status.as_str()),
                "Import is already running or finished"
            );
            let state = job
                .import
                .as_ref()
                .context("This is not a saved-file import")?;
            anyhow::ensure!(
                !state.finished && serde_json::to_vec(&state.files)? == serde_json::to_vec(&files)?,
                "Reselect the same files and folder paths to resume this import"
            );
            job.config = config;
            job.status = "importing".into();
            job.error = None;
            record.save(&job).await?;
            return Ok(job.clone());
        }
        let job = self.create_record(config, false).await?;
        let record = self.record(&job.id).await?;
        tokio::fs::create_dir_all(record.dir.join("uploads")).await?;
        record
            .change(|j| {
                j.import = Some(ImportState {
                    file_count: files.len(),
                    files,
                    finished: false,
                    whole_warc: false,
                });
                j.log(
                    "info",
                    "Importing local files. No pages or assets will be fetched from the internet.",
                );
            })
            .await?;
        self.get(&job.id).await
    }
    pub async fn import_offset(&self, id: &str, path: &str) -> Result<u64> {
        let record = self.record(id).await?;
        let job = record.job.lock().await;
        let state = job.import.as_ref().context("This is not an import")?;
        anyhow::ensure!(
            !state.finished && job.status == "importing",
            "Import is not accepting files"
        );
        let file = state
            .files
            .iter()
            .find(|f| f.path == path)
            .context("File is not in the approved import")?;
        let size = tokio::fs::metadata(record.dir.join("uploads").join(web::key(path)))
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        anyhow::ensure!(
            size <= file.size,
            "Saved import exceeds the expected file size"
        );
        Ok(size)
    }
    pub async fn import_chunk_base64(
        &self,
        id: &str,
        path: &str,
        offset: u64,
        data: &str,
    ) -> Result<u64> {
        use base64::Engine;
        anyhow::ensure!(data.len() <= 349_528, "Import chunk exceeds 256 KB");
        let bytes = base64::engine::general_purpose::STANDARD.decode(data)?;
        self.import_chunk(id, path, offset, &bytes).await
    }
    pub async fn import_chunk(
        &self,
        id: &str,
        path: &str,
        offset: u64,
        bytes: &[u8],
    ) -> Result<u64> {
        anyhow::ensure!(
            bytes.len() <= 256 * 1024,
            "Import chunks must be at most 256 KB"
        );
        let record = self.record(id).await?;
        let mut job = record.job.lock().await;
        anyhow::ensure!(
            !record.running.load(std::sync::atomic::Ordering::SeqCst),
            "Import files are already being prepared"
        );
        let state = job.import.as_ref().context("This is not an import")?;
        anyhow::ensure!(
            !state.finished && job.status == "importing",
            "Import is not accepting files"
        );
        let expected = state
            .files
            .iter()
            .find(|f| f.path == path)
            .context("Unapproved import path")?
            .size;
        let target = record.dir.join("uploads").join(web::key(path));
        let size = tokio::fs::metadata(&target)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        anyhow::ensure!(
            size == offset
                && size
                    .checked_add(bytes.len() as u64)
                    .is_some_and(|n| n <= expected),
            "Import offset or size changed; reselect files to resume"
        );
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(target)
            .await?;
        file.write_all(bytes).await?;
        file.sync_all().await?;
        job.progress.label = format!("Importing {path}");
        job.progress.received = size + bytes.len() as u64;
        job.progress.expected = Some(expected);
        job.progress.updated_at = crate::now();
        record.save(&job).await?;
        Ok(size + bytes.len() as u64)
    }
    pub async fn cancel_import(&self, id: &str) -> Result<()> {
        let record = self.record(id).await?;
        let mut job = record.job.lock().await;
        anyhow::ensure!(
            job.import.as_ref().is_some_and(|s| !s.finished) && job.phase == "import",
            "Only unfinished imports can be canceled"
        );
        record
            .import_cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
        job.status = "failed".into();
        job.error = Some(
            "Import interrupted. Reselect the same files to continue, or delete this attempt."
                .into(),
        );
        record.save(&job).await
    }
    pub async fn finish_import(&self, id: &str) -> Result<Job> {
        use std::sync::atomic::Ordering;
        let record = self.record(id).await?;
        let job = record.job.lock().await;
        anyhow::ensure!(
            job.status == "importing" && job.import.as_ref().is_some_and(|s| !s.finished),
            "Import is not ready to finalize"
        );
        anyhow::ensure!(
            record
                .running
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok(),
            "Import is already being prepared"
        );
        struct Guard(std::sync::Arc<crate::Record>);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.running.store(false, Ordering::SeqCst);
            }
        }
        let _guard = Guard(record.clone());
        record.import_cancel.store(false, Ordering::SeqCst);
        drop(job);
        let result = self.finalize_import(id).await;
        if let Err(error) = &result {
            record.import_cancel.store(true, Ordering::SeqCst);
            let _ = record
                .change(|job| {
                    if job.status != "paused" && job.import.as_ref().is_some_and(|s| !s.finished) {
                        job.status = "failed".into();
                        job.error = Some(format!("{error:#}"));
                        job.log("error", format!("Import failed: {error:#}"));
                    }
                })
                .await;
        }
        result
    }
    async fn finalize_import(&self, id: &str) -> Result<Job> {
        use std::sync::atomic::Ordering;
        let record = self.record(id).await?;
        let job = record.job.lock().await.clone();
        anyhow::ensure!(job.status == "importing", "Import is not ready to finalize");
        let import = job.import.clone().context("Not an import")?;
        let mut payloads = Vec::new();
        let mut names = BTreeSet::new();
        let mut total = 0u64;
        let cap = job
            .config
            .capture
            .max_download_mb
            .map(|mb| mb * 1024 * 1024)
            .unwrap_or(MAX_TOTAL)
            .min(MAX_TOTAL);
        let mut warc_budget_used = 0u64;
        let whole_warc = import
            .files
            .iter()
            .all(|f| crate::warc_import::recognized(&f.path));
        for file in &import.files {
            let path = record.dir.join("uploads").join(web::key(&file.path));
            anyhow::ensure!(
                tokio::fs::metadata(&path).await?.len() == file.size,
                "Import file is incomplete: {}",
                file.path
            );
            let files = if crate::warc_import::recognized(&file.path) {
                record
                    .progress(
                        format!("Reading WARC: {}", file.path),
                        0,
                        Some(file.size),
                        "bytes",
                    )
                    .await;
                let output = record.dir.join("expanded").join(web::key(&file.path));
                let file_cap = job.config.max_resource_mb * 1024 * 1024;
                let control = record.clone();
                let gzip = file.path.to_ascii_lowercase().ends_with(".gz");
                let remaining = cap.saturating_sub(warc_budget_used);
                let expanded = tokio::task::spawn_blocking(move || {
                    crate::warc_import::expand(&path, gzip, &output, remaining, file_cap, control)
                })
                .await??;
                warc_budget_used += expanded.budget_used;
                expanded.payloads
            } else if extension(&file.path) == "zip" {
                record
                    .progress(format!("Extracting ZIP: {}", file.path), 0, None, "files")
                    .await;
                let output = record.dir.join("expanded").join(web::key(&file.path));
                let file_cap = job.config.max_resource_mb * 1024 * 1024;
                let control = record.clone();
                tokio::task::spawn_blocking(move || {
                    expand_zip(&path, &output, cap, file_cap, control)
                })
                .await??
            } else {
                vec![Payload {
                    path: file.path.clone(),
                    file: path,
                    size: file.size,
                    url: None,
                    mime: None,
                }]
            };
            for payload in files {
                total = total
                    .checked_add(payload.size)
                    .context("Import size overflow")?;
                anyhow::ensure!(
                    total <= cap && payloads.len() < MAX_FILES,
                    "Expanded import exceeds its file or storage budget"
                );
                anyhow::ensure!(
                    names.insert(payload.path.to_lowercase()),
                    "Imports contain overlapping file paths: {}",
                    payload.path
                );
                payloads.push(payload);
            }
        }
        let mut entries = Vec::new();
        let mut ordinary_items = 0usize;
        let mut resources = BTreeMap::new();
        tokio::fs::create_dir_all(record.dir.join("media")).await?;
        for (index, payload) in payloads.iter().enumerate() {
            anyhow::ensure!(
                !record.import_cancel.load(Ordering::SeqCst),
                "Import interrupted; reselect saved files to resume"
            );
            record
                .progress(
                    "Preparing captured pages and assets",
                    index as u64,
                    Some(payloads.len() as u64),
                    "files",
                )
                .await;
            let url = match &payload.url {
                Some(url) => web::http_url(url)?,
                None => local_url(&payload.path)?,
            };
            let id = web::key(url.as_str());
            let ext = extension(&payload.path);
            let mime = payload.mime.clone().unwrap_or_else(|| {
                mime_guess::from_path(&payload.path)
                    .first_or_octet_stream()
                    .to_string()
            });
            if ["text/html", "application/xhtml+xml"].contains(&mime.as_str()) {
                anyhow::ensure!(
                    payload.size <= 8 * 1024 * 1024,
                    "Imported HTML exceeds the 8 MB parsing limit: {}",
                    payload.path
                );
                let bytes = tokio::fs::read(&payload.file).await?;
                let html = String::from_utf8_lossy(&bytes);
                let data = web::inspect(&html, &url);
                let cache = record.dir.join("cache").join(format!("{id}.html"));
                // Large WARC imports can share their already-decoded UTF-8
                // bodies with the cache instead of copying them a second time.
                let linked =
                    if payload.url.is_some() && matches!(html, std::borrow::Cow::Borrowed(_)) {
                        let _ = tokio::fs::remove_file(&cache).await;
                        tokio::fs::hard_link(&payload.file, &cache).await.is_ok()
                    } else {
                        false
                    };
                if !linked {
                    tokio::fs::write(&cache, html.as_bytes()).await?;
                }
                tokio::fs::write(
                    record.dir.join("cache").join(format!("{id}.url")),
                    url.as_str(),
                )
                .await?;
                ordinary_items += usize::from(payload.url.is_none());
                entries.push(Entry {
                    id,
                    url: url.to_string(),
                    title: data.title,
                    depth: 0,
                    selected: job.config.capture.page(&url),
                    state: "done".into(),
                    size: Some(payload.size),
                    duration: None,
                    error: None,
                    // Re-read saved HTML during packaging to keep large WARC
                    // checkpoints and review requests small.
                    text: if payload.url.is_some() {
                        data.text.chars().take(256).collect()
                    } else {
                        data.text
                    },
                    assets: data.assets,
                    media_file: None,
                    media_audio: None,
                    thumbnail_url: None,
                    thumbnail_file: None,
                    embedded_in: vec![],
                    rendered: None,
                    subtitles: vec![],
                    transcript: vec![],
                    transcript_file: None,
                    subtitle_policy: None,
                    subtitle_error: None,
                });
            } else {
                let path = format!(
                    "{id}.{}",
                    if ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
                        ext.as_str()
                    } else {
                        "bin"
                    }
                );
                let allowed =
                    job.config.capture.asset_url(&url) && job.config.capture.asset_mime(&mime);
                let cache_path = record.dir.join("cache").join(&path);
                let _ = tokio::fs::remove_file(&cache_path).await;
                if tokio::fs::hard_link(&payload.file, &cache_path)
                    .await
                    .is_err()
                {
                    tokio::fs::copy(&payload.file, &cache_path).await?;
                }
                if mime.starts_with("audio/") || mime.starts_with("video/") {
                    let media_path = record.dir.join("media").join(&path);
                    let _ = tokio::fs::remove_file(&media_path).await;
                    if tokio::fs::hard_link(&cache_path, &media_path)
                        .await
                        .is_err()
                    {
                        tokio::fs::copy(&cache_path, &media_path).await?;
                    }
                    ordinary_items += usize::from(payload.url.is_none());
                    entries.push(Entry {
                        id: id.clone(),
                        url: url.to_string(),
                        title: payload.path.clone(),
                        depth: 0,
                        selected: allowed,
                        state: "done".into(),
                        size: Some(payload.size),
                        duration: None,
                        error: None,
                        text: String::new(),
                        assets: vec![],
                        media_file: Some(path.clone()),
                        media_audio: None,
                        thumbnail_url: None,
                        thumbnail_file: None,
                        embedded_in: vec![],
                        rendered: None,
                        subtitles: vec![],
                        transcript: vec![],
                        transcript_file: None,
                        subtitle_policy: None,
                        subtitle_error: None,
                    });
                }

                let dependencies = if mime == "text/css" {
                    anyhow::ensure!(
                        payload.size <= 2 * 1024 * 1024,
                        "Imported CSS exceeds safe parsing size"
                    );
                    web::css_urls(&tokio::fs::read_to_string(&payload.file).await?, &url)
                } else {
                    vec![]
                };
                resources.insert(
                    url.to_string(),
                    Resource {
                        url: url.to_string(),
                        path,
                        state: if allowed { "done" } else { "excluded" }.into(),
                        error: (!allowed).then(|| "Excluded by capture settings".into()),
                        mime,
                        dependencies,
                        etag: None,
                        last_modified: None,
                        size: payload.size,
                    },
                );
            }
        }
        anyhow::ensure!(
            !entries.is_empty(),
            "No HTML, video, or audio files were found in this import"
        );
        anyhow::ensure!(ordinary_items<=job.config.max_items,"Import contains more pages/media than the item limit; raise the limit or choose fewer files");
        record
            .progress("Saving review and cleaning up temporary files", 0, None, "")
            .await;
        record.change(|job| {
            if job.status != "importing" { return; }
            job.entries=entries; job.resources=resources; job.queue.clear(); job.visited.clear(); job.bytes=total;
            let import=job.import.as_mut().unwrap(); import.finished=true; import.whole_warc=whole_warc; import.file_count=import.files.len(); import.files.clear(); job.phase="scan".into(); job.status="review".into(); job.error=None;
            let count=job.entries.len();
            job.log("info",format!("Imported {count} items. Review your selection before building. External resources will be reported as missing, never fetched."));
        }).await?;
        anyhow::ensure!(
            !record.import_cancel.load(Ordering::SeqCst),
            "Import interrupted; reselect saved files to resume"
        );
        let _ = tokio::fs::remove_dir_all(record.dir.join("uploads")).await;
        let _ = tokio::fs::remove_dir_all(record.dir.join("expanded")).await;
        self.get(id).await
    }
}
