use crate::{NewJob, Record};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoQuality {
    Small,
    #[default]
    Balanced,
    High,
}
impl VideoQuality {
    pub fn height(self) -> u32 {
        match self {
            Self::Small => 360,
            Self::Balanced => 720,
            Self::High => 1080,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CaptureOptions {
    pub auto_render_javascript: bool,
    pub render_javascript: bool,
    pub render_timeout_secs: u64,
    pub embedded_videos: bool,
    pub discover_sitemaps: bool,
    pub include_urls: Vec<String>,
    pub seed_urls: Vec<String>,
    pub exclude_urls: Vec<String>,
    pub images: bool,
    pub styles: bool,
    pub fonts: bool,
    pub media: bool,
    pub documents: bool,
    pub max_download_mb: Option<u64>,
    pub video_quality: VideoQuality,
    pub subtitles: bool,
    pub subtitle_language: String,
    pub auto_captions: bool,
}
impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            auto_render_javascript: true,
            render_javascript: false,
            render_timeout_secs: 45,
            embedded_videos: true,
            discover_sitemaps: false,
            include_urls: vec![],
            seed_urls: vec![],
            exclude_urls: vec![],
            images: true,
            styles: true,
            fonts: true,
            media: true,
            documents: true,
            max_download_mb: None,
            video_quality: Default::default(),
            subtitles: true,
            subtitle_language: "original".into(),
            auto_captions: true,
        }
    }
}
#[derive(Debug)]
pub struct Excluded(pub String);
impl std::fmt::Display for Excluded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Excluded {}
#[derive(Debug)]
pub struct BudgetReached;
impl std::fmt::Display for BudgetReached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Download budget reached. Increase the budget in Edit crawl, or build with available files.")
    }
}
impl std::error::Error for BudgetReached {}

// Small, bounded URL globs: '*' is any text and '?' is one character. Matching
// paths separately keeps /news/* useful without forcing an origin into rules.
pub fn matches(pattern: &str, url: &url::Url) -> bool {
    let expression = format!(
        "^{}$",
        regex::escape(pattern)
            .replace("\\*", ".*")
            .replace("\\?", ".")
    );
    regex::Regex::new(&expression).is_ok_and(|r| {
        r.is_match(if pattern.starts_with('/') {
            url.path()
        } else {
            url.as_str()
        })
    })
}
pub fn linked_asset(url: &url::Url) -> bool {
    [
        "png", "jpg", "jpeg", "webp", "gif", "svg", "avif", "ico", "mp4", "webm", "mov", "mp3",
        "m4a", "ogg", "wav", "pdf", "epub", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "zip",
        "zim", "css", "woff", "woff2", "ttf", "otf",
    ]
    .contains(
        &url.path()
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
    )
}
impl CaptureOptions {
    pub fn validate(&self, source: &url::Url) -> Result<()> {
        anyhow::ensure!(
            (10..=180).contains(&self.render_timeout_secs),
            "Browser rendering timeout must be 10–180 seconds"
        );
        anyhow::ensure!(!self.render_javascript || self.max_download_mb.is_none(), "Browser rendering cannot enforce a strict download budget. Disable rendering or clear the download budget.");
        anyhow::ensure!(
            self.seed_urls.len() <= 1000,
            "Use at most 1000 starting URLs"
        );
        for seed in &self.seed_urls {
            let url = crate::web::http_url(seed)?;
            anyhow::ensure!(
                !self.excluded(&url),
                "A starting URL matches an exclude rule"
            );
        }
        anyhow::ensure!(
            self.include_urls.len() <= 50 && self.exclude_urls.len() <= 50,
            "Use at most 50 include and exclude rules"
        );
        anyhow::ensure!(
            self.include_urls
                .iter()
                .chain(&self.exclude_urls)
                .all(|r| !r.trim().is_empty() && r.len() <= 512),
            "URL rules must contain 1–512 characters"
        );
        anyhow::ensure!(
            !self.excluded(source),
            "The source URL matches an exclude rule"
        );
        anyhow::ensure!(
            self.max_download_mb
                .is_none_or(|n| (1..=1_048_576).contains(&n)),
            "Download budget must be 1–1048576 MB, or blank for unlimited"
        );
        anyhow::ensure!(
            self.subtitle_language == "original"
                || (self.subtitle_language.len() <= 35
                    && self
                        .subtitle_language
                        .split('-')
                        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric()))),
            "Use original or a subtitle language code such as en or pt-BR"
        );
        Ok(())
    }
    pub fn excluded(&self, url: &url::Url) -> bool {
        self.exclude_urls.iter().any(|r| matches(r, url))
    }
    pub fn page(&self, url: &url::Url) -> bool {
        !self.excluded(url)
            && (self.include_urls.is_empty() || self.include_urls.iter().any(|r| matches(r, url)))
    }
    pub fn asset_url(&self, url: &url::Url) -> bool {
        if self.excluded(url) {
            return false;
        }
        let path = url.path().to_ascii_lowercase();
        let ext = path.rsplit('.').next().unwrap_or("");
        match ext {
            "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "avif" | "ico" | "bmp" => self.images,
            "css" => self.styles,
            "woff" | "woff2" | "ttf" | "otf" | "eot" => self.fonts,
            "mp4" | "webm" | "mp3" | "m4a" | "ogg" | "wav" | "mov" | "m3u8" => self.media,
            "zip" | "zim" | "pdf" | "epub" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" => {
                self.documents
            }
            _ => true,
        }
    }
    pub fn asset_mime(&self, mime: &str) -> bool {
        let mime = mime
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if mime.starts_with("image/") {
            self.images
        } else if mime == "text/css" {
            self.styles
        } else if mime.starts_with("font/")
            || [
                "application/font-woff",
                "application/vnd.ms-fontobject",
                "application/x-font-ttf",
                "application/x-font-opentype",
            ]
            .contains(&mime.as_str())
        {
            self.fonts
        } else if mime.starts_with("audio/") || mime.starts_with("video/") {
            self.media
        } else if mime == "application/zip"
            || mime == "application/x-zim"
            || mime == "application/pdf"
            || mime == "application/epub+zip"
            || mime.starts_with("application/vnd.openxmlformats")
        {
            self.documents
        } else {
            true
        }
    }
}
impl Record {
    pub async fn consume_download(&self, size: u64) -> Result<()> {
        let mut job = self.job.lock().await;
        let next = job
            .downloaded_bytes
            .checked_add(size)
            .ok_or(BudgetReached)?;
        if job
            .config
            .capture
            .max_download_mb
            .is_some_and(|mb| next > mb * 1024 * 1024)
        {
            return Err(BudgetReached.into());
        }
        job.downloaded_bytes = next;
        Ok(())
    }
    pub async fn remaining_download(&self) -> Option<u64> {
        let job = self.job.lock().await;
        job.config
            .capture
            .max_download_mb
            .map(|mb| (mb * 1024 * 1024).saturating_sub(job.downloaded_bytes))
    }
}
impl NewJob {
    pub fn starting_pages(&self) -> Vec<crate::PendingPage> {
        let mut seen = std::collections::BTreeSet::new();
        std::iter::once(&self.url)
            .chain(&self.capture.seed_urls)
            .filter(|url| seen.insert((*url).clone()))
            .map(|url| crate::PendingPage {
                url: url.clone(),
                depth: 0,
            })
            .collect()
    }
    pub fn contains_url(&self, url: &url::Url) -> bool {
        std::iter::once(&self.url)
            .chain(&self.capture.seed_urls)
            .filter_map(|value| crate::web::http_url(value).ok())
            .any(|root| crate::web::in_scope(&root, url))
    }
    pub fn page_allowed(&self, value: &str) -> bool {
        crate::web::http_url(value).is_ok_and(|u| self.capture.page(&u))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rules_match_paths_or_full_urls_without_regex_injection_and_excludes_win() {
        let mut options = CaptureOptions::default();
        options.include_urls = vec!["/news/*".into()];
        options.exclude_urls = vec!["*/ads/*".into(), "/news/private*".into()];
        assert!(options.page(&url::Url::parse("https://example.test/news/story").unwrap()));
        assert!(!options.page(&url::Url::parse("https://example.test/news/private-post").unwrap()));
        assert!(!options.page(&url::Url::parse("https://example.test/about").unwrap()));
        options.fonts = false;
        options.images = false;
        assert!(!options.asset_url(&url::Url::parse("https://cdn.test/font.woff2?v=1").unwrap()));
        assert!(!options.asset_mime("image/jpeg; charset=utf-8"));
        assert!(options.asset_mime("text/css"));
        assert!(matches(
            "/file[1].html",
            &url::Url::parse("https://example.test/file[1].html").unwrap()
        ));
        assert!(!matches(
            "/file[1].html",
            &url::Url::parse("https://example.test/file1.html").unwrap()
        ));
    }
}
