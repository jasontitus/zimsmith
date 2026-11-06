use crate::{model::*, Record};
use anyhow::{bail, Context, Result};
use futures_util::{stream::FuturesUnordered, StreamExt};
use reqwest::{Client, StatusCode};
use scraper::{Html, Selector};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncSeekExt, AsyncWriteExt},
    sync::{Mutex, Semaphore},
};
use url::Url;

struct HostSlot {
    gate: Mutex<()>,
    timing: Mutex<HostTiming>,
    transfers: Semaphore,
}

#[derive(Default)]
struct HostTiming {
    last_activity: Option<Instant>,
    cooldown_until: Option<Instant>,
}
static SLOTS: OnceLock<Mutex<HashMap<String, Arc<HostSlot>>>> = OnceLock::new();

async fn host_slot(url: &Url) -> Arc<HostSlot> {
    SLOTS
        .get_or_init(Default::default)
        .lock()
        .await
        .entry(url.origin().ascii_serialization())
        .or_insert_with(|| {
            Arc::new(HostSlot {
                gate: Mutex::new(()),
                timing: Mutex::new(HostTiming::default()),
                transfers: Semaphore::new(MAX_CDN_PARALLEL_REQUESTS),
            })
        })
        .clone()
}

// Shared with the browser engine. Match exact hosts, never a CDN-looking
// prefix, public-cloud suffix, or filename on an otherwise unknown site.
pub fn is_static_cdn(url: &Url) -> bool {
    static HOSTS: OnceLock<Vec<String>> = OnceLock::new();
    let hosts = HOSTS.get_or_init(|| {
        #[derive(serde::Deserialize)]
        struct Cdns {
            hosts: Vec<String>,
        }
        serde_json::from_str::<Cdns>(include_str!("../../../assets/static-cdns.json"))
            .expect("valid bundled static-CDN policy")
            .hosts
    });
    url.host_str().is_some_and(|host| {
        hosts
            .iter()
            .any(|known| known == host.trim_end_matches('.'))
    })
}

pub fn http_url(value: &str) -> Result<Url> {
    let mut url = Url::parse(value.trim()).context("Enter a complete http:// or https:// URL")?;
    anyhow::ensure!(
        ["http", "https"].contains(&url.scheme()) && url.host_str().is_some(),
        "Only HTTP and HTTPS URLs are supported"
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "URLs containing credentials are not supported"
    );
    url.set_fragment(None);
    Ok(url)
}

/// Only entry-point URLs are repaired; discovered links still use strict resolution.
pub fn source_url(value: &str) -> Result<Url> {
    let value = value.trim();
    anyhow::ensure!(
        !value.is_empty(),
        "Enter a website such as cnn.com or a complete HTTP/HTTPS URL"
    );
    anyhow::ensure!(
        !value.chars().any(char::is_control),
        "The URL contains an unexpected control character"
    );
    let lower = value.to_ascii_lowercase();
    let authority = value.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = regex::Regex::new(r"^[a-zA-Z0-9._-]+:[0-9]+$")
        .unwrap()
        .is_match(authority);
    let scheme = regex::Regex::new(r"^[a-zA-Z][a-zA-Z0-9+.-]*:")
        .unwrap()
        .is_match(value);
    anyhow::ensure!(
        !scheme || lower.starts_with("http:") || lower.starts_with("https:") || host_port,
        "Only HTTP and HTTPS URLs are supported"
    );
    let candidate = if value.starts_with("//") {
        format!("https:{value}")
    } else if lower.starts_with("http:") || lower.starts_with("https:") || value.contains("://") {
        value.to_owned()
    } else {
        format!("https://{value}")
    };
    http_url(&candidate)
}

/// A first-page redirect may add/remove www or upgrade HTTPS, never change site/scope.
fn canonical_start_redirect(source: &Url, target: &Url) -> bool {
    let same_site = source
        .host_str()
        .zip(target.host_str())
        .is_some_and(|(a, b)| {
            a.strip_prefix("www.").unwrap_or(a) == b.strip_prefix("www.").unwrap_or(b)
        });
    same_site
        && source.port() == target.port()
        && (source.scheme() == target.scheme()
            || (source.scheme() == "http" && target.scheme() == "https"))
        && source.path().trim_end_matches('/') == target.path().trim_end_matches('/')
}

pub fn key(url: &str) -> String {
    format!("{:x}", Sha256::digest(url.as_bytes()))[..24].into()
}
pub fn resolve(base: &Url, value: &str) -> Option<Url> {
    http_url(base.join(value.trim()).ok()?.as_str()).ok()
}
pub fn in_scope(root: &Url, url: &Url) -> bool {
    root.origin() == url.origin()
        && (root.path() == "/"
            || url.path() == root.path()
            || url
                .path()
                .starts_with(&format!("{}/", root.path().trim_end_matches('/'))))
}

struct Policy {
    text: String,
    delay: Duration,
}

#[derive(Debug)]
struct RetryFailure {
    status: u16,
    retry_after: bool,
    url: String,
    attempts: u32,
}
impl std::fmt::Display for RetryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "HTTP {} after {} attempt(s): {}",
            self.status, self.attempts, self.url
        )
    }
}
impl std::error::Error for RetryFailure {}

#[derive(Debug)]
struct UnavailablePolicy {
    message: String,
    passive_fallback: bool,
}
impl std::fmt::Display for UnavailablePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}
impl std::error::Error for UnavailablePolicy {}

fn passive_policy_failure(error: &anyhow::Error) -> bool {
    error.chain().any(|error| {
        error
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|e| e.is_timeout() && !e.is_connect())
            || error
                .downcast_ref::<RetryFailure>()
                .is_some_and(|e| e.status >= 500 && e.status != 503 && !e.retry_after)
    })
}

fn passive_cdn_asset(url: &Url) -> bool {
    if !is_static_cdn(url) {
        return false;
    }
    let path = url.path().to_ascii_lowercase();
    let ext = path.rsplit('.').next().unwrap_or("");
    [
        "png", "jpg", "jpeg", "gif", "svg", "webp", "avif", "ico", "css", "woff", "woff2", "ttf",
        "otf", "eot",
    ]
    .contains(&ext)
        || (url.host_str() == Some("fonts.googleapis.com")
            && matches!(path.as_str(), "/css" | "/css2"))
}

fn passive_cdn_mime(mime: &str) -> bool {
    mime.starts_with("image/")
        || mime.starts_with("font/")
        || mime.starts_with("text/css")
        || mime.starts_with("application/font")
        || mime.starts_with("application/x-font")
        || mime.starts_with("application/vnd.ms-fontobject")
}

fn host_failure(error: &anyhow::Error) -> bool {
    error.chain().any(|error| {
        error.is::<UnavailablePolicy>()
            || error.is::<RetryFailure>()
            || error
                .downcast_ref::<reqwest::Error>()
                .is_some_and(|e| e.is_timeout() || e.is_connect())
    })
}

struct ActiveAssetRequest<'a>(&'a AtomicUsize);

impl<'a> ActiveAssetRequest<'a> {
    fn start(record: &'a Record) -> Self {
        let count = &record.active_asset_requests;
        let active = count.fetch_add(1, Ordering::Relaxed) + 1;
        record
            .peak_asset_requests
            .fetch_max(active, Ordering::Relaxed);
        Self(count)
    }
}

impl Drop for ActiveAssetRequest<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct Transfer<'a> {
    file: Option<&'a mut tokio::fs::File>,
    batch: bool,
    validators: Option<(Option<String>, Option<String>)>,
}

pub struct Network<'a> {
    record: &'a Record,
    client: Client,
    delay: Duration,
    cap: u64,
    robots: Mutex<HashMap<String, Policy>>,
    robots_timeout: Duration,
    cdn_notices: Mutex<std::collections::BTreeSet<String>>,
    serial: Mutex<()>,
}

impl<'a> Network<'a> {
    pub async fn new(record: &'a Record) -> Result<Self> {
        let job = record.job.lock().await;
        Ok(Self {
            record,
            client: Client::builder()
                .user_agent("ZimStudio/0.1 (offline archive crawler)")
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(60))
                .build()?,
            delay: Duration::from_millis(job.config.delay_ms),
            cap: job.config.max_resource_mb * 1024 * 1024,
            robots: Mutex::new(HashMap::new()),
            robots_timeout: Duration::from_secs(15),
            cdn_notices: Default::default(),
            serial: Mutex::new(()),
        })
    }
    pub async fn declared_sitemaps(&self, url: &Url) -> Vec<String> {
        self.robots
            .lock()
            .await
            .get(&url.origin().ascii_serialization())
            .map(|p| {
                p.text
                    .lines()
                    .filter_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.trim()
                            .eq_ignore_ascii_case("sitemap")
                            .then(|| value.trim().to_owned())
                    })
                    .take(8)
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn with_cap(mut self, bytes: u64) -> Self {
        self.cap = self.cap.min(bytes);
        self
    }
    async fn raw(
        &self,
        url: &Url,
        delay: Duration,
        limit: u64,
        robots: bool,
        asset: bool,
        conditional: reqwest::header::HeaderMap,
        transfer: &mut Transfer<'_>,
        parallel: bool,
    ) -> Result<(StatusCode, reqwest::header::HeaderMap, Vec<u8>)> {
        if self.record.remaining_download().await == Some(0) {
            return Err(crate::capture::BudgetReached.into());
        }
        let _serial = if parallel {
            None
        } else {
            Some(self.serial.lock().await)
        };
        let slot = host_slot(url).await;
        let attempts = if robots || parallel { 1 } else { 5 };
        for attempt in 0..attempts {
            wait_saved_cooldown(self.record).await?;
            let _permit = slot
                .transfers
                .acquire_many(if parallel {
                    1
                } else {
                    MAX_CDN_PARALLEL_REQUESTS as u32
                })
                .await
                .expect("open transfer semaphore");
            self.record
                .change(|j| {
                    j.requests += 1;
                    if transfer.batch && !robots {
                        j.progress.label = "Downloading CDN assets".into();
                        return;
                    }
                    j.progress.label = if robots {
                        format!("Checking robots.txt: {url} (15s timeout)")
                    } else {
                        format!("Downloading {url} · attempt {} of {attempts}", attempt + 1)
                    };
                    j.progress.received = 0;
                    j.progress.expected = None;
                    j.progress.updated_at = now();
                })
                .await?;
            pace(url, delay).await;
            let request = self.client.get(url.clone()).headers(conditional.clone());
            let request = if robots {
                request.timeout(self.robots_timeout)
            } else {
                request
            };
            let _active = transfer
                .file
                .is_some()
                .then(|| ActiveAssetRequest::start(self.record));
            let response = request.send().await;
            note_response(url).await;
            match response {
                Ok(mut response) => {
                    let status = response.status();
                    let headers = response.headers().clone();
                    if status == StatusCode::TOO_MANY_REQUESTS
                        || status == StatusCode::SERVICE_UNAVAILABLE
                        || status.is_server_error()
                    {
                        let wait = retry_delay(
                            headers.get("retry-after").and_then(|v| v.to_str().ok()),
                            attempt,
                        );
                        cooldown(url, wait).await;
                        self.record
                            .change(|j| {
                                j.retry_at =
                                    Some(j.retry_at.unwrap_or(0).max(
                                        now().saturating_add(wait.as_secs()).saturating_add(1),
                                    ));
                                j.log(
                                    "warning",
                                    format!(
                                        "HTTP {status}. Backing off for {}s: {url}",
                                        wait.as_secs()
                                    ),
                                );
                            })
                            .await?;
                        // Keep the final rate-limit cooldown too: another job
                        // or a quick Resume must not immediately hit this host.
                        if attempt + 1 == attempts {
                            return Err(RetryFailure {
                                status: status.as_u16(),
                                retry_after: headers.contains_key("retry-after"),
                                url: url.to_string(),
                                attempts,
                            }
                            .into());
                        }
                        continue;
                    }
                    if asset && status.is_success() {
                        let mime = headers
                            .get("content-type")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("");
                        if !self.record.job.lock().await.config.capture.asset_mime(mime) {
                            return Err(crate::capture::Excluded(format!(
                                "Excluded by file-type settings: {url}"
                            ))
                            .into());
                        }
                        anyhow::ensure!(
                            !parallel || passive_cdn_mime(mime) || matches!(mime, "" | "application/octet-stream"),
                            "The CDN returned {mime} instead of an image, stylesheet or font: {url}"
                        );
                    }
                    if transfer.file.is_some() && !status.is_success() {
                        return Ok((status, headers, Vec::new()));
                    }
                    if self
                        .record
                        .remaining_download()
                        .await
                        .is_some_and(|remaining| response.content_length().unwrap_or(0) > remaining)
                    {
                        return Err(crate::capture::BudgetReached.into());
                    }
                    anyhow::ensure!(
                        response.content_length().unwrap_or(0) <= limit,
                        "Resource exceeds {} MB limit: {}",
                        limit / 1024 / 1024,
                        url
                    );
                    let mut body = Vec::new();
                    let mut received = 0u64;
                    let mut file = transfer.file.as_deref_mut().filter(|_| status.is_success());
                    if let Some(file) = file.as_mut() {
                        file.set_len(0).await?;
                        file.rewind().await?;
                    }
                    let total = response.content_length();
                    if !transfer.batch {
                        self.record.transfer_progress(0, total).await;
                    }
                    let mut last_update = std::time::Instant::now();
                    while let Some(chunk) = response.chunk().await? {
                        anyhow::ensure!(
                            received + chunk.len() as u64 <= limit,
                            "Resource exceeded size limit: {url}"
                        );
                        self.record.consume_download(chunk.len() as u64).await?;
                        received += chunk.len() as u64;
                        if let Some(file) = file.as_mut() {
                            file.write_all(&chunk).await?;
                        } else {
                            body.extend_from_slice(&chunk);
                        }
                        if !transfer.batch && last_update.elapsed() >= Duration::from_millis(500) {
                            self.record.transfer_progress(received, total).await;
                            last_update = std::time::Instant::now();
                        }
                    }
                    if let Some(file) = file {
                        file.flush().await?;
                    }
                    if !transfer.batch {
                        self.record.transfer_progress(received, total).await;
                    }
                    self.record
                        .change(|j| {
                            j.bytes += received;
                        })
                        .await?;
                    return Ok((status, headers, body));
                }
                Err(error) if attempt + 1 == attempts => return Err(error.into()),
                Err(_) => {
                    let wait = retry_delay(None, attempt);
                    self.record
                        .change(|j| {
                            j.retry_at = Some(
                                j.retry_at
                                    .unwrap_or(0)
                                    .max(now().saturating_add(wait.as_secs()).saturating_add(1)),
                            );
                            j.log(
                                "warning",
                                format!("Connection error. Retrying in {}s: {url}", wait.as_secs()),
                            );
                        })
                        .await?;
                }
            }
        }
        unreachable!()
    }
    async fn check_robots(&self, url: &Url, baseline: Duration) -> Result<Duration> {
        use crate::robots_cache::{self, Checkpoint};
        let origin = url.origin().ascii_serialization();
        let mut robots = self.robots.lock().await;
        if robots.contains_key(&origin) {
            return self.check_robots_uncached(url, baseline, &mut robots).await;
        }
        let cached = robots_cache::load(self.record, &origin).await;
        if let Some(cached) = &cached {
            if let Some(text) = &cached.text {
                let delay = Duration::from_millis(cached.delay_ms);
                cooldown(url, delay).await;
                robots.insert(
                    origin.clone(),
                    Policy {
                        text: text.clone(),
                        delay,
                    },
                );
                return self.check_robots_uncached(url, baseline, &mut robots).await;
            }
            if cached.error.is_some() || cached.attempts >= 2 {
                let message = format!("Cannot safely read {origin}/robots.txt: {}. Retry in {}s, or pause and build with the available files.", cached.error.as_deref().unwrap_or("The previous policy checks were interrupted"), cached.valid_until.saturating_sub(now()));
                return Err(UnavailablePolicy {
                    message,
                    passive_fallback: cached.passive_fallback,
                }
                .into());
            }
        }
        // Persist before requesting: repeated pause/resume cannot reset an
        // unreachable host's retry budget. One interrupted check can retry once.
        let attempts = cached.as_ref().map_or(1, |c| c.attempts + 1);
        robots_cache::save(
            self.record,
            &Checkpoint {
                origin: origin.clone(),
                valid_until: now() + 60,
                text: None,
                delay_ms: 0,
                error: None,
                attempts,
                passive_fallback: false,
            },
        )
        .await?;
        let result = self.check_robots_uncached(url, baseline, &mut robots).await;
        let passive_fallback = result.as_ref().err().is_some_and(passive_policy_failure);
        let policy = robots.get(&origin);
        let unavailable = policy.is_none();
        robots_cache::save(
            self.record,
            &Checkpoint {
                origin,
                valid_until: now() + if policy.is_some() { 3600 } else { 60 },
                text: policy.map(|p| p.text.clone()),
                delay_ms: policy.map_or(0, |p| p.delay.as_millis() as u64),
                error: if policy.is_some() {
                    None
                } else {
                    result.as_ref().err().map(|e| format!("{e:#}"))
                },
                attempts,
                passive_fallback,
            },
        )
        .await?;
        result.map_err(|error| {
            if unavailable {
                UnavailablePolicy {
                    message: format!("{error:#}"),
                    passive_fallback,
                }
                .into()
            } else {
                error
            }
        })
    }

    async fn check_robots_uncached(
        &self,
        url: &Url,
        baseline: Duration,
        robots: &mut HashMap<String, Policy>,
    ) -> Result<Duration> {
        let origin = url.origin().ascii_serialization();
        if !robots.contains_key(&origin) {
            let mut robots_url = Url::parse(&format!("{origin}/robots.txt"))?;
            let mut result = None;
            for _ in 0..=5 {
                let (status, headers, body) = self
                    .raw(
                        &robots_url,
                        baseline,
                        1024 * 1024,
                        true,
                        false,
                        Default::default(),
                        &mut Transfer::default(),
                        false,
                    )
                    .await?;
                if status.is_redirection() {
                    let location = headers
                        .get("location")
                        .context("robots.txt redirect without Location")?
                        .to_str()?;
                    let target =
                        resolve(&robots_url, location).context("Invalid robots.txt redirect")?;
                    anyhow::ensure!(
                        target.origin() == url.origin(),
                        "Cross-origin robots.txt redirect requires manual review"
                    );
                    robots_url = target;
                    continue;
                }
                let text = if status == StatusCode::NOT_FOUND || status == StatusCode::GONE {
                    String::new()
                } else if status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED {
                    "User-agent: *\nDisallow: /".into()
                } else {
                    anyhow::ensure!(
                        status.is_success(),
                        "Cannot safely read robots.txt: {status}"
                    );
                    String::from_utf8_lossy(&body).into_owned()
                };
                // Conservatively honor the longest declared crawl delay.
                let crawl_delay = text
                    .lines()
                    .filter_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if name.trim().eq_ignore_ascii_case("crawl-delay") {
                            value
                                .split('#')
                                .next()?
                                .trim()
                                .parse::<f64>()
                                .ok()
                                .filter(|v| v.is_finite() && *v >= 0.0)
                        } else {
                            None
                        }
                    })
                    .fold(0.0, f64::max);
                let declared = Duration::from_secs_f64(crawl_delay.min(86400.0));
                if declared > baseline {
                    // The robots fetch used the baseline. Apply a newly learned
                    // longer delay before the first asset as well as later ones.
                    cooldown(url, declared).await;
                }
                result = Some(Policy {
                    text,
                    delay: declared,
                });
                break;
            }
            robots.insert(
                origin.clone(),
                result.context("Too many robots.txt redirects")?,
            );
        }
        let policy = &robots[&origin];
        let mut matcher = robotstxt::DefaultMatcher::default();
        anyhow::ensure!(
            matcher.one_agent_allowed_by_robots(&policy.text, "ZimStudio", url.as_str()),
            "Blocked by robots.txt: {url}"
        );
        Ok(baseline.max(policy.delay))
    }
    pub async fn get(&self, url: &Url) -> Result<(Url, String, Vec<u8>)> {
        self.get_with_purpose(url, false, false, &mut Transfer::default())
            .await
    }
    pub async fn rendering_delay(&self, url: &Url) -> Result<Duration> {
        self.check_robots(url, self.delay).await
    }
    pub async fn get_asset(&self, url: &Url) -> Result<(Url, String, Vec<u8>)> {
        self.get_asset_with_transfer(url, &mut Transfer::default())
            .await
    }
    async fn get_asset_with_transfer(
        &self,
        url: &Url,
        transfer: &mut Transfer<'_>,
    ) -> Result<(Url, String, Vec<u8>)> {
        let result = self.get_with_purpose(url, true, false, transfer).await;
        if let Err(error) = &result {
            if let Some(alternative) =
                public_font_alternative(url).filter(|_| temporary_source_failure(error))
            {
                self.record.log("warning", "The font proxy is unavailable. Trying the same font families from Google Fonts, with its own robots and rate-limit checks.").await?;
                let batch = transfer.batch;
                transfer.batch = false;
                let result = self
                    .get_with_purpose(&alternative, true, false, transfer)
                    .await;
                transfer.batch = batch;
                return result;
            }
        }
        result
    }
    pub async fn get_same_origin(&self, url: &Url) -> Result<(Url, String, Vec<u8>)> {
        self.get_with_purpose(url, false, true, &mut Transfer::default())
            .await
    }
    async fn get_with_purpose(
        &self,
        url: &Url,
        asset: bool,
        same_origin: bool,
        transfer: &mut Transfer<'_>,
    ) -> Result<(Url, String, Vec<u8>)> {
        let origin = url.origin();
        let mut url = url.clone();
        for _ in 0..=8 {
            if asset && !self.record.job.lock().await.config.capture.asset_url(&url) {
                return Err(crate::capture::Excluded(format!(
                    "Excluded by capture settings: {url}"
                ))
                .into());
            }
            let baseline = if asset && is_static_cdn(&url) {
                Duration::ZERO
            } else {
                self.delay
            };
            let (delay, passive_fallback) = match self.check_robots(&url, baseline).await {
                Ok(delay) => (delay, false),
                Err(error) => {
                    let external = {
                        let job = self.record.job.lock().await;
                        job.config.kind == SourceKind::Website
                            && !job
                                .config
                                .starting_pages()
                                .iter()
                                .filter_map(|p| http_url(&p.url).ok())
                                .any(|root| root.origin() == url.origin())
                    };
                    if !asset
                        || !external
                        || !passive_cdn_asset(&url)
                        || !error
                            .downcast_ref::<UnavailablePolicy>()
                            .is_some_and(|failure| failure.passive_fallback)
                    {
                        return Err(error);
                    }
                    let origin = url.origin().ascii_serialization();
                    if self.cdn_notices.lock().await.insert(origin.clone()) {
                        self.record.log("warning",format!("The policy endpoint at {origin} is unreachable. Fetching this page's linked images, styles and fonts as browser assets. Explicit denials, HTTP failures, rate-limit cooldowns and download limits still apply.")).await?;
                    }
                    (baseline, true)
                }
            };
            let cached = if asset {
                self.record
                    .job
                    .lock()
                    .await
                    .resources
                    .get(url.as_str())
                    .cloned()
            } else {
                None
            };
            let mut conditional = reqwest::header::HeaderMap::new();
            if let Some(resource) = &cached {
                if !self
                    .record
                    .job
                    .lock()
                    .await
                    .config
                    .capture
                    .asset_mime(&resource.mime)
                {
                    return Err(
                        crate::capture::Excluded("Excluded by file-type settings".into()).into(),
                    );
                }
                if tokio::fs::metadata(self.record.dir.join("cache").join(&resource.path))
                    .await
                    .is_ok_and(|m| m.is_file() && m.len() == resource.size && m.len() <= self.cap)
                {
                    if let Some(value) = resource.etag.as_ref().and_then(|s| s.parse().ok()) {
                        conditional.insert(reqwest::header::IF_NONE_MATCH, value);
                    }
                    if let Some(value) =
                        resource.last_modified.as_ref().and_then(|s| s.parse().ok())
                    {
                        conditional.insert(reqwest::header::IF_MODIFIED_SINCE, value);
                    }
                }
            }
            let parallel = transfer.batch
                && url.origin() == origin
                && passive_cdn_asset(&url)
                && delay.is_zero();
            let (status, headers, body) = self
                .raw(
                    &url,
                    delay,
                    self.cap,
                    false,
                    asset,
                    conditional,
                    transfer,
                    parallel,
                )
                .await?;
            if passive_fallback && status.is_success() {
                let mime = headers
                    .get("content-type")
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("");
                anyhow::ensure!(
                    passive_cdn_mime(mime),
                    "The CDN returned {mime} instead of an image, stylesheet or font: {url}"
                );
            }
            if status == StatusCode::NOT_MODIFIED {
                let resource = cached.context("Server returned 304 without a cached asset")?;
                let path = self.record.dir.join("cache").join(&resource.path);
                let (size, body) = if let Some(file) = transfer.file.as_deref_mut() {
                    let mut saved = tokio::fs::File::open(path).await?;
                    anyhow::ensure!(
                        saved.metadata().await?.len() == resource.size && resource.size <= self.cap,
                        "Cached asset is incomplete or exceeds the file limit"
                    );
                    file.set_len(0).await?;
                    file.rewind().await?;
                    let size = tokio::io::copy(&mut saved, file).await?;
                    file.flush().await?;
                    (size, Vec::new())
                } else {
                    let body = tokio::fs::read(path).await?;
                    (body.len() as u64, body)
                };
                anyhow::ensure!(
                    size == resource.size && resource.size <= self.cap,
                    "Cached asset is incomplete or exceeds the file limit"
                );
                let base = tokio::fs::read_to_string(
                    self.record
                        .dir
                        .join("cache")
                        .join(format!("{}.url", resource.path)),
                )
                .await
                .unwrap_or_else(|_| url.to_string());
                return Ok((http_url(&base)?, resource.mime, body));
            }
            if asset && status.is_success() {
                transfer.validators = Some((
                    headers
                        .get("etag")
                        .and_then(|h| h.to_str().ok())
                        .map(str::to_owned),
                    headers
                        .get("last-modified")
                        .and_then(|h| h.to_str().ok())
                        .map(str::to_owned),
                ));
            }
            if status.is_redirection() {
                url = resolve(
                    &url,
                    headers
                        .get("location")
                        .context("Redirect without Location")?
                        .to_str()?,
                )
                .context("Invalid redirect")?;
                anyhow::ensure!(
                    !same_origin || url.origin() == origin,
                    "Cross-origin sitemap redirect ignored before requesting its destination"
                );
                continue;
            }
            anyhow::ensure!(status.is_success(), "HTTP {status}: {url}");
            let mime = headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_string();
            return Ok((url, mime, body));
        }
        bail!("Too many redirects")
    }
}

fn temporary_source_failure(error: &anyhow::Error) -> bool {
    let text = error.to_string();
    static SERVER_STATUS: OnceLock<regex::Regex> = OnceLock::new();
    SERVER_STATUS
        .get_or_init(|| regex::Regex::new(r"(?:^|: |HTTP )(5[0-9]{2})\b").unwrap())
        .is_match(&text)
        || text.contains("operation timed out")
        || error.chain().any(|e| {
            e.downcast_ref::<reqwest::Error>()
                .is_some_and(|e| e.is_timeout() || e.is_connect())
        })
}

fn public_font_alternative(url: &Url) -> Option<Url> {
    if url.host_str() != Some("cdn.zyrosite.com") || url.path() != "/u1/google-fonts/font-faces" {
        return None;
    }
    let families: Vec<_> = url
        .query_pairs()
        .filter(|(k, _)| k == "family")
        .map(|(_, v)| v.into_owned())
        .collect();
    if families.is_empty() || families.len() > 10 || families.iter().any(|f| f.len() > 256) {
        return None;
    }
    let mut target = Url::parse("https://fonts.googleapis.com/css2").ok()?;
    for family in families {
        target.query_pairs_mut().append_pair("family", &family);
    }
    target.query_pairs_mut().append_pair("display", "swap");
    Some(target)
}

pub async fn pace(url: &Url, delay: Duration) {
    let slot = host_slot(url).await;
    // Serialize grants for this origin, while allowing other origins to run.
    // Cooldowns use the separate deadline lock and can extend a sleeping wait.
    let _gate = slot.gate.lock().await;
    loop {
        let wait = {
            let mut timing = slot.timing.lock().await;
            let now = Instant::now();
            let next = timing
                .last_activity
                .map_or(now, |last| last + delay)
                .max(timing.cooldown_until.unwrap_or(now));
            if next <= now {
                timing.last_activity = Some(now);
                return;
            }
            next
        };
        tokio::time::sleep_until(wait.into()).await;
    }
}

pub(crate) async fn note_response(url: &Url) {
    host_slot(url).await.timing.lock().await.last_activity = Some(Instant::now());
}

pub async fn cooldown(url: &Url, delay: Duration) {
    let slot = host_slot(url).await;
    let mut timing = slot.timing.lock().await;
    timing.cooldown_until = timing.cooldown_until.max(Some(Instant::now() + delay));
}

pub async fn wait_saved_cooldown(record: &Record) -> Result<()> {
    loop {
        let until = record.job.lock().await.retry_at;
        let Some(until) = until else {
            return Ok(());
        };
        let remaining = until.saturating_sub(now());
        if remaining > 0 {
            tokio::time::sleep(Duration::from_secs(remaining)).await;
        } else {
            record
                .change(|j| {
                    if j.retry_at == Some(until) {
                        j.retry_at = None;
                    }
                })
                .await?;
        }
    }
}

pub fn retry_delay(value: Option<&str>, attempt: u32) -> Duration {
    let minimum = 2u64.saturating_pow(attempt + 1) + (now() % 3);
    let server = value
        .and_then(|v| {
            v.parse::<u64>().ok().or_else(|| {
                httpdate::parse_http_date(v).ok().map(|date| {
                    date.duration_since(std::time::SystemTime::now())
                        .unwrap_or_default()
                        .as_secs()
                        + 1
                })
            })
        })
        .unwrap_or(0);
    Duration::from_secs(minimum.max(server))
}

pub struct PageData {
    pub title: String,
    pub text: String,
    pub links: Vec<String>,
    pub assets: Vec<String>,
}
pub fn visible_text(document: &Html) -> String {
    document
        .select(&Selector::parse("body").unwrap())
        .next()
        .into_iter()
        .flat_map(|body| body.descendants())
        .filter(|node| {
            !node.ancestors().any(|parent| {
                parent
                    .value()
                    .as_element()
                    .is_some_and(|e| matches!(e.name(), "script" | "style" | "template"))
            })
        })
        .filter_map(|node| node.value().as_text().map(|text| text.text.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(100_000)
        .collect()
}
pub fn inspect(html: &str, base: &Url) -> PageData {
    let prepared = crate::images::prepare(html).unwrap_or_else(|_| html.into());
    let document = Html::parse_document(&prepared);
    let select = |s: &str| Selector::parse(s).unwrap();
    let base = document
        .select(&select("base[href]"))
        .next()
        .and_then(|e| resolve(base, e.value().attr("href")?))
        .unwrap_or_else(|| base.clone());
    let title = document
        .select(&select("title"))
        .next()
        .map(|e| e.text().collect::<String>())
        .unwrap_or_else(|| base.path().into());
    let text = visible_text(&document);
    let links = document
        .select(&select("a[href]"))
        .filter_map(|e| resolve(&base, e.value().attr("href")?))
        .filter(|u| {
            !crate::capture::linked_asset(u)
                && !["js", "xml"].contains(
                    &u.path()
                        .rsplit('.')
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .as_str(),
                )
        })
        .map(|u| u.to_string())
        .collect();
    let mut assets: Vec<_> = document
        .select(&select("a[href]"))
        .filter_map(|e| resolve(&base, e.value().attr("href")?))
        .filter(crate::capture::linked_asset)
        .map(|u| u.to_string())
        .collect();
    for element in document.select(&select(
        "img, source, track[src], video[src], audio[src], video[poster], link[href]",
    )) {
        if element.value().name() == "link"
            && !["stylesheet", "icon", "shortcut icon"]
                .contains(&element.value().attr("rel").unwrap_or(""))
        {
            continue;
        }
        for attr in ["src", "poster", "href"] {
            if let Some(url) = element.value().attr(attr).and_then(|v| resolve(&base, v)) {
                assets.push(url.to_string());
            }
        }
        if element.value().attr("src").is_none() {
            let fallback = element.value().attr("data-src").or_else(|| {
                element
                    .value()
                    .attr("srcset")
                    .and_then(|s| s.split(',').next())
                    .and_then(|s| s.split_whitespace().next())
            });
            if let Some(url) = fallback.and_then(|v| resolve(&base, v)) {
                assets.push(url.to_string());
            }
        }
    }
    for element in document.select(&select("[style], style")) {
        let css = element
            .value()
            .attr("style")
            .map(str::to_string)
            .unwrap_or_else(|| element.inner_html());
        assets.extend(css_urls(&css, &base));
    }
    assets.sort();
    assets.dedup();
    PageData {
        title: title.trim().into(),
        text: text.chars().take(100_000).collect(),
        links,
        assets,
    }
}

/// Conservative detection of a known HTTP-200 challenge, not keyword blocking.
pub fn is_challenge(html: &str) -> bool {
    let document = Html::parse_document(html);
    let title = document
        .select(&Selector::parse("title").unwrap())
        .next()
        .map(|e| e.text().collect::<String>().trim().to_ascii_lowercase())
        .unwrap_or_default();
    (title == "just a moment..."
        || title == "just a moment…"
        || title == "attention required! | cloudflare")
        && (html.contains("/cdn-cgi/challenge-platform/")
            || html.contains("_cf_chl_opt")
            || html.contains("cf-chl-widget"))
}

#[cfg(test)]
#[test]
fn detects_known_verification_pages_without_keyword_false_positives() {
    assert!(is_challenge(
        "<title>Just a moment...</title><script src='/cdn-cgi/challenge-platform/start'></script>"
    ));
    assert!(!is_challenge(
        "<title>How CAPTCHA works</title><p>Verification examples.</p>"
    ));
    assert!(!is_challenge(
        "<title>Just a moment...</title><p>A poem.</p>"
    ));
}

pub async fn scan(record: &Record) -> Result<()> {
    let network = Network::new(record).await?.with_cap(8 * 1024 * 1024);
    crate::sitemap::discover(record).await?;
    loop {
        let snapshot = record.job.lock().await.clone();
        let Some(page) = snapshot.queue.first().cloned() else {
            break;
        };
        if snapshot.entries.len() >= snapshot.config.max_items {
            break;
        }
        let page_url = http_url(&page.url)?;
        let mut root = std::iter::once(&snapshot.config.url)
            .chain(&snapshot.config.capture.seed_urls)
            .filter_map(|u| http_url(u).ok())
            .find(|u| in_scope(u, &page_url))
            .context("Page is outside the starting URL scopes")?;
        record
            .progress(
                format!("Scanning {}", page.url),
                snapshot.entries.len() as u64,
                None,
                "pages",
            )
            .await;
        let result = async {
            let (mut url, mime, mut body) = network.get(&http_url(&page.url)?).await?;
            anyhow::ensure!(!mime.contains("html") || !is_challenge(&String::from_utf8_lossy(&body)), "The site returned a browser verification challenge instead of content. Pause and retry later, or import a saved page.");
            let mut rendered = None;
            let dynamic = mime.contains("html")
                && crate::snapshot::needs_javascript(&Html::parse_document(&String::from_utf8_lossy(&body)));
            let automatic = snapshot.config.capture.auto_render_javascript && dynamic;
            let can_render = record.renderer.read().await.is_some() && snapshot.config.capture.max_download_mb.is_none();
            if dynamic && !snapshot.config.capture.render_javascript && !(automatic && can_render) {
                record.change(|job| {
                    job.scan_truncated = true;
                    job.javascript_needed = true;
                    job.log("warning", format!("Dynamic content detected at {url}. Enable JavaScript and rescan to discover missing content. Rendering requires an installed app and a cleared download budget. This static scan is incomplete; review before building."));
                }).await?;
            }
            if (snapshot.config.capture.render_javascript || (automatic && can_render)) && mime.contains("html") {
                if !snapshot.config.capture.render_javascript {
                    record.log("info", format!("Dynamic content detected; automatically rendering {url}")).await?;
                }
                // Validate a redirect before executing any source JavaScript.
                anyhow::ensure!(in_scope(&root, &url) || (page.depth == 0 && canonical_start_redirect(&root, &url)), "The source redirected outside the requested scope. Use its final URL explicitly.");
                let delay = network.rendering_delay(&url).await?;
                let captured = crate::render::capture(record, &url, &String::from_utf8_lossy(&body), delay).await?;
                let effective = http_url(&captured.url)?;
                anyhow::ensure!(in_scope(&url, &effective) || in_scope(&root, &effective), "JavaScript navigation left the requested scope: {effective}");
                url = effective; body = captured.html.into_bytes(); rendered = Some(captured.info);
            }
            Ok::<_, anyhow::Error>((url, mime, body, rendered))
        }.await;
        match result {
            Ok((final_url, mime, body, rendered)) => {
                let adopt = page.depth == 0
                    && page.url == root.as_str()
                    && final_url != root
                    && canonical_start_redirect(&root, &final_url);
                if adopt {
                    root = final_url.clone();
                    record
                        .change(|j| {
                            if page.url == j.config.url {
                                j.config.url = final_url.to_string();
                            } else if let Some(seed) = j
                                .config
                                .capture
                                .seed_urls
                                .iter_mut()
                                .find(|u| **u == page.url)
                            {
                                *seed = final_url.to_string();
                            }
                            j.log(
                                "info",
                                format!("Using the website's canonical URL: {final_url}"),
                            );
                        })
                        .await?;
                }
                anyhow::ensure!(in_scope(&root, &final_url), "The source redirected outside the requested scope to {final_url}. Use that URL explicitly.");
                let entry_url = if adopt {
                    final_url.to_string()
                } else {
                    page.url.clone()
                };
                if !mime.contains("text/html") && !mime.contains("application/xhtml") {
                    record
                        .change(|j| {
                            j.visited.push(page.url.clone());
                            j.queue.remove(0);
                            j.log("warning", format!("Skipped non-HTML page: {}", page.url));
                        })
                        .await?;
                    continue;
                }
                let html = String::from_utf8_lossy(&body).into_owned();
                let data = inspect(&html, &final_url);
                let id = key(&entry_url);
                tokio::fs::write(record.dir.join("cache").join(format!("{id}.html")), &html)
                    .await?;
                // Store the effective document URL for correct relative URL resolution.
                tokio::fs::write(
                    record.dir.join("cache").join(format!("{id}.url")),
                    final_url.as_str(),
                )
                .await?;
                record
                    .change(|j| {
                        j.visited.push(page.url.clone());
                        if entry_url != page.url {
                            j.visited.push(entry_url.clone());
                        }
                        j.queue.remove(0);
                        if page.depth < j.config.max_depth {
                            for link in data.links {
                                if j.queue.len() + j.visited.len() >= j.config.max_items * 10 {
                                    break;
                                }
                                if http_url(&link)
                                    .map(|u| j.config.contains_url(&u))
                                    .unwrap_or(false)
                                    && j.config.page_allowed(&link)
                                    && !j.visited.contains(&link)
                                    && !j.queue.iter().any(|p| p.url == link)
                                {
                                    j.queue.push(PendingPage {
                                        url: link,
                                        depth: page.depth + 1,
                                    });
                                }
                            }
                        }
                        j.log("info", format!("Discovered {}", data.title));
                        let selected = j.config.page_allowed(&entry_url);
                        j.entries.push(Entry {
                            id,
                            url: entry_url.clone(),
                            title: data.title,
                            depth: page.depth,
                            selected,
                            state: "pending".into(),
                            size: Some(body.len() as u64),
                            duration: None,
                            error: None,
                            text: data.text,
                            assets: data.assets,
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
                            rendered,
                        });
                        crate::embedded::register(j, &html, &entry_url, page.depth);
                    })
                    .await?;
            }
            Err(error) => {
                let message = format!("{error:#}");
                record
                    .change(|j| {
                        j.queue.remove(0);
                        j.visited.push(page.url.clone());
                        j.log("warning", message.clone());
                        j.entries.push(Entry {
                            id: key(&page.url),
                            title: page.url.clone(),
                            url: page.url,
                            depth: page.depth,
                            selected: false,
                            state: "scan_failed".into(),
                            size: None,
                            duration: None,
                            error: Some(message),
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
                            rendered: None,
                        });
                    })
                    .await?;
            }
        }
    }
    record
        .change(|j| {
            if !j.queue.is_empty() {
                j.scan_truncated = true;
                j.log(
                    "warning",
                    format!(
                        "Stopped at the {} item scan limit. Remaining URLs were not scanned.",
                        j.config.max_items
                    ),
                );
            }
        })
        .await?;
    let job = record.job.lock().await;
    anyhow::ensure!(
        job.entries.iter().any(|e| e.state != "scan_failed"),
        "No accessible HTML pages found. Check the source URL and activity log."
    );
    Ok(())
}

fn needed_resources(job: &Job) -> std::collections::BTreeSet<String> {
    let mut needed = std::collections::BTreeSet::new();
    if !job.config.include_assets {
        return needed;
    }
    let mut todo: Vec<_> = job
        .entries
        .iter()
        .filter(|e| e.selected)
        .flat_map(|e| e.assets.clone())
        .collect();
    while let Some(url) = todo.pop() {
        if !needed.insert(url.clone()) {
            continue;
        }
        if let Some(resource) = job.resources.get(&url).filter(|r| r.state == "done") {
            todo.extend(resource.dependencies.clone());
        }
    }
    needed
}

struct FetchedResource {
    file: atomic_write_file::AtomicWriteFile,
    final_url: Url,
    mime: String,
    size: u64,
    validators: Option<(Option<String>, Option<String>)>,
}

async fn fetch_resource(
    network: &Network<'_>,
    resource: &Resource,
    batch: bool,
) -> Result<FetchedResource> {
    let file = atomic_write_file::AtomicWriteFile::open(
        network.record.dir.join("cache").join(&resource.path),
    )?;
    let mut output = tokio::fs::File::from_std(file.as_file().try_clone()?);
    let mut transfer = Transfer {
        file: Some(&mut output),
        batch,
        validators: None,
    };
    let requested = http_url(&resource.url)?;
    let (final_url, mime, _) = network
        .get_asset_with_transfer(&requested, &mut transfer)
        .await?;
    let validators = if final_url == requested {
        transfer.validators
    } else {
        // A redirect target's validators are not valid for the original URL.
        Some((None, None))
    };
    let size = output.metadata().await?.len();
    drop(output);
    Ok(FetchedResource {
        file,
        final_url,
        mime,
        size,
        validators,
    })
}

async fn save_resource(record: &Record, resource: &Resource, asset: FetchedResource) -> Result<()> {
    let FetchedResource {
        file,
        final_url,
        mime,
        size,
        validators,
    } = asset;
    file.commit()?;
    let cache = record.dir.join("cache");
    let dependencies = if mime.contains("text/css") {
        let bytes = tokio::fs::read(cache.join(&resource.path)).await?;
        css_urls(&String::from_utf8_lossy(&bytes), &final_url)
    } else {
        vec![]
    };
    tokio::fs::write(
        cache.join(format!("{}.url", resource.path)),
        final_url.as_str(),
    )
    .await?;
    record
        .change(|j| {
            let r = j.resources.get_mut(&resource.url).unwrap();
            r.state = "done".into();
            r.error = None;
            r.mime = mime;
            r.size = size;
            if let Some((etag, last_modified)) = validators {
                r.etag = etag;
                r.last_modified = last_modified;
            }
            r.dependencies = dependencies.clone();
            for url in dependencies {
                add_resource(j, &url);
            }
        })
        .await
}

pub async fn download(record: &Record, parallel_requests: usize) -> Result<()> {
    let network = Network::new(record).await?;
    download_with_network(record, &network, parallel_requests).await
}

async fn download_with_network(
    record: &Record,
    network: &Network<'_>,
    parallel_requests: usize,
) -> Result<()> {
    repair_saved_stylesheets(record).await?;
    let job = record.job.lock().await.clone();
    record
        .change(|j| {
            for resource in j.resources.values_mut() {
                if !http_url(&resource.url).is_ok_and(|u| j.config.capture.asset_url(&u))
                    || !j.config.capture.asset_mime(&resource.mime)
                {
                    resource.state = "excluded".into();
                    resource.error = Some("Excluded by capture settings".into());
                }
            }
        })
        .await?;
    if job.config.include_assets {
        record
            .change(|j| {
                for entry in &job.entries {
                    if !entry.selected {
                        continue;
                    }
                    for url in &entry.assets {
                        add_resource(j, url);
                    }
                }
            })
            .await?;
        let site_origins: std::collections::BTreeSet<_> = job
            .config
            .starting_pages()
            .into_iter()
            .map(|p| p.url)
            .chain(
                job.entries
                    .iter()
                    .filter(|e| e.embedded_in.is_empty() && e.media_file.is_none())
                    .map(|e| e.url.clone()),
            )
            .filter_map(|url| {
                http_url(&url)
                    .ok()
                    .map(|u| u.origin().ascii_serialization())
            })
            .collect();
        let parallel_requests = parallel_requests.clamp(1, MAX_CDN_PARALLEL_REQUESTS);
        let mut parallel = parallel_requests > 1
            && job.config.capture.max_download_mb.is_none()
            && job.retry_at.is_none_or(|until| until <= now());
        record.peak_asset_requests.store(0, Ordering::Relaxed);
        let mut group_origin = None;
        let mut in_flight = HashSet::new();
        let mut transfers = FuturesUnordered::new();
        loop {
            let batch = {
                let mut job = record.job.lock().await;
                let needed = needed_resources(&job);
                let pending = || {
                    job.resources.values().filter(|r| {
                        r.state == "pending"
                            && needed.contains(&r.url)
                            && !in_flight.contains(&r.url)
                    })
                };
                if transfers.is_empty() {
                    group_origin = pending()
                        .next()
                        .and_then(|r| http_url(&r.url).ok())
                        .filter(|u| {
                            parallel
                                && passive_cdn_asset(u)
                                && !site_origins.contains(&u.origin().ascii_serialization())
                        })
                        .map(|u| u.origin());
                }
                let batch: Vec<_> = if let Some(origin) = &group_origin {
                    pending()
                        .filter(|r| {
                            http_url(&r.url)
                                .is_ok_and(|u| &u.origin() == origin && passive_cdn_asset(&u))
                        })
                        .take(parallel_requests - transfers.len())
                        .cloned()
                        .collect()
                } else {
                    pending().take(1).cloned().collect()
                };
                if transfers.is_empty() && batch.is_empty() {
                    break;
                }
                let done = job.resources.values().filter(|r| r.state == "done").count() as u64;
                let total = Some(job.resources.len() as u64);
                if transfers.is_empty() {
                    job.progress = Progress {
                        label: if group_origin.is_some() {
                            "Downloading CDN assets".into()
                        } else {
                            format!("Downloading {}", batch[0].url)
                        },
                        completed: done,
                        total,
                        unit: "files".into(),
                        updated_at: now(),
                        active_asset_requests: Some(0),
                        ..Progress::default()
                    };
                } else {
                    job.progress.completed = done;
                    job.progress.total = total;
                    job.progress.updated_at = now();
                }
                batch
            };
            let concurrent = group_origin.is_some();
            for resource in batch {
                in_flight.insert(resource.url.clone());
                transfers.push(async move {
                    let result = fetch_resource(network, &resource, concurrent).await;
                    (resource, result)
                });
            }

            let Some((resource, result)) = transfers.next().await else {
                break;
            };
            in_flight.remove(&resource.url);
            match result {
                Ok(asset) => save_resource(record, &resource, asset).await?,
                Err(error)
                    if concurrent
                        && error
                            .downcast_ref::<RetryFailure>()
                            .is_some_and(|e| e.attempts == 1) =>
                {
                    // Cancel the active requests and keep unfinished files pending.
                    parallel = false;
                    transfers.clear();
                    in_flight.clear();
                    record.log("info", "The CDN asked us to slow down. Continuing one file at a time after the cooldown.").await?;
                }
                Err(error) if error.is::<std::io::Error>() => {
                    return Err(error.context("Could not save the linked asset to disk"));
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    let stop = host_failure(&error);
                    record
                        .change(|j| {
                            let r = j.resources.get_mut(&resource.url).unwrap();
                            r.state = if error.is::<crate::capture::Excluded>() {
                                "excluded"
                            } else {
                                "failed"
                            }
                            .into();
                            r.error = Some(message.clone());
                            j.log("warning", message);
                        })
                        .await?;
                    if stop {
                        return Err(error.context(format!("Downloads stopped for {}. Remaining files are still queued; Resume retries after the server cooldown.", http_url(&resource.url)?.origin().ascii_serialization())));
                    }
                }
            }
        }
        record.job.lock().await.progress.active_asset_requests = None;
        let peak = record.peak_asset_requests.load(Ordering::Relaxed);
        if peak > 1 {
            record
                .log(
                    "info",
                    format!("Linked asset downloads reached {peak} simultaneous requests."),
                )
                .await?;
        }
    }
    record
        .change(|j| {
            let needed = needed_resources(j);
            j.resources.retain(|url, _| needed.contains(url));
            for e in &mut j.entries {
                if e.selected && e.embedded_in.is_empty() {
                    e.state = "done".into();
                }
            }
        })
        .await?;
    let failed = record
        .job
        .lock()
        .await
        .resources
        .values()
        .filter(|r| r.state == "failed")
        .count();
    anyhow::ensure!(
        failed == 0,
        "{failed} assets failed. Resume to retry, or change the selection and build again."
    );
    Ok(())
}

/// Recompute legacy CSS references locally so resumes/rebuilds do not retain
/// bogus URLs extracted from strings inside embedded SVGs by older versions.
pub async fn repair_saved_stylesheets(record: &Record) -> Result<()> {
    let snapshot = record.job.lock().await.clone();
    let needed = needed_resources(&snapshot);
    let mut changes = Vec::new();
    for resource in snapshot
        .resources
        .values()
        .filter(|r| r.state == "done" && r.mime.contains("text/css") && needed.contains(&r.url))
    {
        let path = record.dir.join("cache").join(&resource.path);
        if tokio::fs::metadata(&path).await?.len() > 8 * 1024 * 1024 {
            continue;
        }
        let css = tokio::fs::read_to_string(&path).await?;
        let base = tokio::fs::read_to_string(
            record
                .dir
                .join("cache")
                .join(format!("{}.url", resource.path)),
        )
        .await
        .unwrap_or_else(|_| resource.url.clone());
        let dependencies = css_urls(&css, &http_url(&base)?);
        if dependencies != resource.dependencies {
            changes.push((resource.url.clone(), dependencies));
        }
    }
    if changes.is_empty() {
        return Ok(());
    }
    record.change(|job| {
        for (url, dependencies) in changes { if let Some(resource) = job.resources.get_mut(&url) { resource.dependencies = dependencies; } }
        let needed = needed_resources(job);
        job.resources.retain(|url,_|needed.contains(url));
        job.log("info","Updated saved stylesheet references locally; embedded SVG masks do not require downloads.");
    }).await
}

fn add_resource(job: &mut Job, url: &str) {
    if job.resources.len() >= 50_000 {
        return;
    }
    job.resources.entry(url.into()).or_insert_with(|| {
        let ext = http_url(url)
            .ok()
            .and_then(|u| {
                u.path()
                    .rsplit('.')
                    .next()
                    .filter(|s| s.len() <= 8 && s.chars().all(|c| c.is_ascii_alphanumeric()))
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "bin".into());
        Resource {
            url: url.into(),
            path: format!("{}.{}", key(url), ext),
            state: "pending".into(),
            error: None,
            mime: String::new(),
            dependencies: vec![],
            etag: None,
            last_modified: None,
            size: 0,
        }
    });
}

pub fn css_urls(css: &str, base: &Url) -> Vec<String> {
    crate::css::references(css)
        .into_iter()
        .filter(|r| !crate::css::local(&r.value))
        .filter_map(|r| resolve(base, &r.value))
        .map(|u| u.to_string())
        .collect()
}

pub fn rewrite_css(css: &str, base: &Url, paths: &BTreeMap<String, String>) -> String {
    let mut out = css.to_string();
    for reference in crate::css::references(css).into_iter().rev() {
        if crate::css::local(&reference.value) {
            continue;
        }
        let Some(path) =
            resolve(base, &reference.value).and_then(|u| paths.get(u.as_str()).cloned())
        else {
            continue;
        };
        let fragment = base
            .join(&reference.value)
            .ok()
            .and_then(|u| u.fragment().map(|f| format!("#{f}")))
            .unwrap_or_default();
        let value = format!("{path}{fragment}");
        let replacement = if reference.import {
            format!("\"{value}\"")
        } else {
            format!("url(\"{value}\")")
        };
        out.replace_range(reference.start..reference.end, &replacement);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn font_proxy_recovery_is_narrow_and_does_not_override_denials_or_rate_limits() {
        let url=http_url("https://cdn.zyrosite.com/u1/google-fonts/font-faces?family=Inter:wght@400&family=Mukta:wght@400").unwrap();
        let target = public_font_alternative(&url).unwrap();
        assert_eq!(target.host_str(), Some("fonts.googleapis.com"));
        assert_eq!(
            target.query_pairs().filter(|(k, _)| k == "family").count(),
            2
        );
        assert!(public_font_alternative(
            &http_url(
                "https://cdn.zyrosite.com.example.test/u1/google-fonts/font-faces?family=Inter"
            )
            .unwrap()
        )
        .is_none());
        for message in [
            "429 Too Many Requests after 5 attempts",
            "Blocked by robots.txt",
            "HTTP 403 Forbidden",
            "Download budget reached",
        ] {
            assert!(!temporary_source_failure(&anyhow::anyhow!(message)));
        }
        for message in [
            "522 after 1 attempt",
            "Cannot safely read robots.txt: 503 after 1 attempt",
            "operation timed out",
        ] {
            assert!(temporary_source_failure(&anyhow::anyhow!(message)));
        }
    }
    #[tokio::test]
    async fn robots_failure_is_bounded_and_cached_across_networks_and_expiry_allows_retry() {
        use axum::{response::IntoResponse, routing::get, Router};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let app = Router::new().route(
            "/robots.txt",
            get(move || {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    (StatusCode::from_u16(522).unwrap(), "unavailable").into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = http_url(&format!(
            "http://{}/font.woff2",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let temp = tempfile::tempdir().unwrap();
        let job: Job = serde_json::from_value(serde_json::json!({
            "id":"robots-failure", "config":{"url":"https://example.test/", "title":"Test", "kind":"website", "max_items":1, "max_depth":0, "delay_ms":1000, "max_resource_mb":1, "language":"eng", "description":"Test", "creator":"Test", "include_assets":true},
            "status":"paused", "phase":"download", "created_at":1, "updated_at":1,
            "entries":[], "queue":[], "visited":[], "resources":{}, "logs":[],
            "error":null, "output":null, "output_bytes":null, "retry_at":null, "requests":0, "bytes":0
        })).unwrap();
        let record = Record {
            renderer: Default::default(),
            job: Mutex::new(job),
            stop: Default::default(),
            running: Default::default(),
            import_cancel: Default::default(),
            active_asset_requests: Default::default(),
            peak_asset_requests: Default::default(),
            dir: temp.path().to_owned(),
        };
        for _ in 0..3 {
            let network = Network::new(&record).await.unwrap();
            assert!(network.check_robots(&url, Duration::ZERO).await.is_err());
        }
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "resume must not reset the failed policy retry budget"
        );
        assert!(record
            .job
            .lock()
            .await
            .progress
            .label
            .contains("robots.txt"));
        let origin = url.origin().ascii_serialization();
        let mut cached = crate::robots_cache::load(&record, &origin).await.unwrap();
        cached.valid_until = 0;
        crate::robots_cache::save(&record, &cached).await.unwrap();
        // Clear the saved cooldown only in this fixture to avoid a real backoff wait.
        record.job.lock().await.retry_at = None;
        let network = Network::new(&record).await.unwrap();
        network
            .check_robots(&url, Duration::ZERO)
            .await
            .unwrap_err();
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        // A paused in-flight check may retry once, then must honor the checkpoint.
        cached.valid_until = now() + 60;
        cached.error = None;
        cached.attempts = 2;
        crate::robots_cache::save(&record, &cached).await.unwrap();
        assert!(Network::new(&record)
            .await
            .unwrap()
            .check_robots(&url, Duration::ZERO)
            .await
            .unwrap_err()
            .to_string()
            .contains("interrupted"));
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        server.abort();
    }

    #[test]
    fn static_cdn_matching_is_exact() {
        for (url, expected) in [
            ("https://fonts.googleapis.com/css2?family=Inter", true),
            ("https://FONTS.GSTATIC.COM./s/inter/font.woff2", true),
            ("https://cdn.jsdelivr.net/npm/example/style.css", true),
            ("https://example.com/font.woff2", false),
            ("https://fonts.gstatic.com.evil.test/font.woff2", false),
            ("https://cdn.example.com/font.woff2", false),
            ("https://tenant.cloudfront.net/font.woff2", false),
        ] {
            assert_eq!(is_static_cdn(&http_url(url).unwrap()), expected, "{url}");
        }
    }

    #[tokio::test]
    async fn queued_requests_observe_a_cooldown_extended_while_waiting() {
        let url = http_url("https://queued-cooldown.test/").unwrap();
        pace(&url, Duration::from_millis(100)).await;
        let queued_url = url.clone();
        let queued = tokio::spawn(async move {
            pace(&queued_url, Duration::from_millis(100)).await;
            Instant::now()
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let extended = Instant::now();
        cooldown(&url, Duration::from_millis(200)).await;
        let granted = tokio::time::timeout(Duration::from_secs(3), queued)
            .await
            .unwrap()
            .unwrap();
        assert!(granted.duration_since(extended) >= Duration::from_millis(200));
    }

    #[tokio::test]
    async fn retry_uses_the_edited_spacing_instead_of_the_previous_delay() {
        let url = http_url("https://edited-spacing.test/").unwrap();
        pace(&url, Duration::from_secs(30)).await;
        note_response(&url).await;
        let edited_at = Instant::now();
        tokio::time::timeout(
            Duration::from_secs(3),
            pace(&url, Duration::from_millis(40)),
        )
        .await
        .unwrap();
        assert!(edited_at.elapsed() >= Duration::from_millis(40));
    }

    #[tokio::test]
    async fn cdn_assets_are_fast_but_pages_robots_and_429_still_control_pacing() {
        use axum::{
            extract::{Path, State},
            http::{HeaderMap, StatusCode},
            response::IntoResponse,
            routing::get,
            Router,
        };
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        #[derive(Default)]
        struct Hits {
            requests: Mutex<Vec<(String, String, Instant)>>,
            limited: AtomicUsize,
        }
        async fn robots(headers: HeaderMap) -> &'static str {
            let host = headers["host"].to_str().unwrap();
            if host.starts_with("fonts.googleapis.com:") {
                "User-agent: *\nCrawl-delay: 0.15\nAllow: /"
            } else if host.starts_with("fonts.bunny.net:") {
                "User-agent: *\nDisallow: /"
            } else {
                "User-agent: *\nAllow: /"
            }
        }
        async fn asset(
            State(hits): State<Arc<Hits>>,
            Path(path): Path<String>,
            headers: HeaderMap,
        ) -> axum::response::Response {
            let host = headers["host"]
                .to_str()
                .unwrap()
                .split(':')
                .next()
                .unwrap()
                .to_owned();
            hits.requests
                .lock()
                .await
                .push((host, path.clone(), Instant::now()));
            if path == "limited" && hits.limited.fetch_add(1, Ordering::SeqCst) == 0 {
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    [("Retry-After", "1")],
                    "wait",
                )
                    .into_response();
            }
            "asset bytes".into_response()
        }
        let hits = Arc::new(Hits::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new()
            .route("/robots.txt", get(robots))
            .route("/{path}", get(asset))
            .with_state(hits.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let temp = tempfile::tempdir().unwrap();
        let job: Job = serde_json::from_value(serde_json::json!({
            "id":"cdn-test", "config": {"url":"https://example.com/", "title":"CDN pacing", "kind":"website", "max_items":1, "max_depth":0, "delay_ms":30000, "max_resource_mb":1, "language":"eng", "description":"Pacing test", "creator":"Test", "include_assets":true},
            "status":"paused", "phase":"download", "auto_start_item_limit":null,
            "scan_truncated":false, "archive_template":"atlas", "created_at":now(), "updated_at":now(),
            "entries":[], "queue":[], "visited":[], "resources":{}, "logs":[],
            "error":null, "output":null, "output_bytes":null, "retry_at":null, "requests":0, "bytes":0
        })).unwrap();
        let record = Record {
            renderer: Default::default(),
            job: Mutex::new(job),
            stop: Default::default(),
            running: Default::default(),
            import_cancel: Default::default(),
            active_asset_requests: Default::default(),
            peak_asset_requests: Default::default(),
            dir: temp.path().to_owned(),
        };
        let mut network = Network::new(&record).await.unwrap();
        let hosts = [
            "fonts.gstatic.com",
            "fonts.googleapis.com",
            "fonts.bunny.net",
            "cdn.jsdelivr.net",
        ];
        let mut client = Client::builder().no_proxy();
        for host in hosts {
            client = client.resolve(host, address);
        }
        network.client = client.build().unwrap();
        let url = |host: &str, path: &str| {
            http_url(&format!("http://{host}:{}/{path}", address.port())).unwrap()
        };
        // A 30-second configured delay must not stall ordinary CDN font assets.
        tokio::time::timeout(Duration::from_secs(5), async {
            network
                .get_asset(&url("fonts.gstatic.com", "one"))
                .await
                .unwrap();
            network
                .get_asset(&url("fonts.gstatic.com", "two"))
                .await
                .unwrap();
        })
        .await
        .expect("CDN assets were artificially delayed");
        // A page on that same recognized host still waits for the configured gap.
        let page = url("fonts.gstatic.com", "page");
        assert!(
            tokio::time::timeout(Duration::from_millis(100), network.get(&page))
                .await
                .is_err()
        );
        network
            .get_asset(&url("fonts.googleapis.com", "one"))
            .await
            .unwrap();
        network
            .get_asset(&url("fonts.googleapis.com", "two"))
            .await
            .unwrap();
        let times: Vec<_> = hits
            .requests
            .lock()
            .await
            .iter()
            .filter(|(host, _, _)| host == "fonts.googleapis.com")
            .map(|(_, _, time)| *time)
            .collect();
        assert!(times[1].duration_since(times[0]) >= Duration::from_millis(140));
        assert!(network
            .get_asset(&url("fonts.bunny.net", "blocked"))
            .await
            .unwrap_err()
            .to_string()
            .contains("robots.txt"));
        assert!(!hits
            .requests
            .lock()
            .await
            .iter()
            .any(|(host, _, _)| host == "fonts.bunny.net"));
        network
            .get_asset(&url("cdn.jsdelivr.net", "limited"))
            .await
            .unwrap();
        let times: Vec<_> = hits
            .requests
            .lock()
            .await
            .iter()
            .filter(|(_, path, _)| path == "limited")
            .map(|(_, _, time)| *time)
            .collect();
        assert_eq!(times.len(), 2);
        assert!(times[1].duration_since(times[0]) >= Duration::from_secs(1));
        server.abort();
        // Policies survive a fresh Network without fetching again, including
        // exclusions and declared Crawl-delay. The server is now unavailable.
        let resumed = Network::new(&record).await.unwrap();
        let delay = resumed
            .check_robots(&url("fonts.googleapis.com", "three"), Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(delay, Duration::from_millis(150));
        assert!(resumed
            .check_robots(&url("fonts.bunny.net", "blocked"), Duration::ZERO)
            .await
            .unwrap_err()
            .to_string()
            .contains("Blocked by robots.txt"));
    }
    #[tokio::test]
    async fn unavailable_external_cdn_policy_recovers_passive_assets_without_relaxing_page_rules() {
        use axum::{extract::State, response::IntoResponse, routing::get, Router};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        #[derive(Default)]
        struct Hits {
            robots: AtomicUsize,
            images: AtomicUsize,
        }
        let hits = Arc::new(Hits::default());
        let app = Router::new()
            .route(
                "/robots.txt",
                get(|State(hits): State<Arc<Hits>>| async move {
                    hits.robots.fetch_add(1, Ordering::SeqCst);
                    (StatusCode::from_u16(522).unwrap(), "unavailable")
                }),
            )
            .route(
                "/{file}",
                get(|State(hits): State<Arc<Hits>>| async move {
                    hits.images.fetch_add(1, Ordering::SeqCst);
                    ([("Content-Type", "image/jpeg")], "image bytes").into_response()
                }),
            )
            .with_state(hits.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let temp = tempfile::tempdir().unwrap();
        let engine = crate::Engine::open(temp.path()).await.unwrap();
        let config:NewJob=serde_json::from_value(serde_json::json!({"url":"https://example.com/","title":"CDN recovery","kind":"website","max_items":10,"max_depth":1,"delay_ms":1000,"max_resource_mb":1,"language":"eng","description":"CDN recovery fixture","creator":"Fixture","include_assets":true})).unwrap();
        let job = engine.create_record(config, false).await.unwrap();
        let record = engine.record(&job.id).await.unwrap();
        let client = || {
            Client::builder()
                .no_proxy()
                .resolve("cdn.zyrosite.com", address)
                .build()
                .unwrap()
        };
        let url = |path: &str| {
            http_url(&format!(
                "http://cdn.zyrosite.com:{}/{path}",
                address.port()
            ))
            .unwrap()
        };
        let mut network = Network::new(&record).await.unwrap();
        network.client = client();
        assert!(network.get_asset(&url("one.jpg")).await.is_ok());
        assert!(network.get_asset(&url("two.jpg")).await.is_ok());
        assert_eq!(hits.robots.load(Ordering::SeqCst), 1);
        assert_eq!(hits.images.load(Ordering::SeqCst), 2);
        assert_eq!(
            record
                .job
                .lock()
                .await
                .logs
                .iter()
                .filter(|log| log.message.contains("as browser assets"))
                .count(),
            1
        );
        // Restarting the network keeps the negative policy classification but does not refetch it.
        let mut resumed = Network::new(&record).await.unwrap();
        resumed.client = client();
        assert!(resumed.get_asset(&url("three.jpg")).await.is_ok());
        assert_eq!(hits.robots.load(Ordering::SeqCst), 1);
        // The exception never applies to pages, API/script/document paths, or this host as a crawl source.
        assert!(resumed.get(&url("page.jpg")).await.is_err());
        for path in ["index.html", "app.js", "catalog.json", "document.pdf"] {
            assert!(resumed.get_asset(&url(path)).await.is_err());
        }
        record.job.lock().await.config.url = url("").to_string();
        assert!(resumed.get_asset(&url("same-origin.jpg")).await.is_err());
        assert_eq!(hits.images.load(Ordering::SeqCst), 3);
        server.abort();
    }

    #[tokio::test]
    async fn cdn_recovery_preserves_denials_rate_limits_retry_after_and_content_types() {
        use axum::{
            extract::{Path, State},
            http::HeaderMap,
            response::IntoResponse,
            routing::get,
            Router,
        };
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let hits = Arc::new(AtomicUsize::new(0));
        async fn robots(headers: HeaderMap) -> axum::response::Response {
            let host = headers["host"].to_str().unwrap().split(':').next().unwrap();
            match host {
                "fonts.bunny.net" => (StatusCode::FORBIDDEN, "denied").into_response(),
                "fonts.gstatic.com" => (
                    StatusCode::TOO_MANY_REQUESTS,
                    [("Retry-After", "1")],
                    "wait",
                )
                    .into_response(),
                "cdnjs.cloudflare.com" => (StatusCode::SERVICE_UNAVAILABLE, "wait").into_response(),
                "unpkg.com" => {
                    (StatusCode::BAD_GATEWAY, [("Retry-After", "1")], "wait").into_response()
                }
                _ => (StatusCode::from_u16(522).unwrap(), "unavailable").into_response(),
            }
        }
        async fn asset(
            State(hits): State<Arc<AtomicUsize>>,
            Path(path): Path<String>,
        ) -> axum::response::Response {
            hits.fetch_add(1, Ordering::SeqCst);
            if path == "wrong.css" {
                (
                    [("Content-Type", "text/html")],
                    "<html>not a stylesheet</html>",
                )
                    .into_response()
            } else {
                ([("Content-Type", "image/jpeg")], "image").into_response()
            }
        }
        let app = Router::new()
            .route("/robots.txt", get(robots))
            .route("/{file}", get(asset))
            .with_state(hits.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        for host in [
            "fonts.bunny.net",
            "fonts.gstatic.com",
            "cdnjs.cloudflare.com",
            "unpkg.com",
            "cdn.zyrosite.com.example.test",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let engine = crate::Engine::open(temp.path()).await.unwrap();
            let config:NewJob=serde_json::from_value(serde_json::json!({"url":"https://example.com/","title":"CDN guard","kind":"website","max_items":1,"max_depth":0,"delay_ms":1000,"max_resource_mb":1,"language":"eng","description":"Guard fixture","creator":"Fixture","include_assets":true})).unwrap();
            let job = engine.create_record(config, false).await.unwrap();
            let record = engine.record(&job.id).await.unwrap();
            let mut network = Network::new(&record).await.unwrap();
            network.client = Client::builder()
                .no_proxy()
                .resolve(host, address)
                .build()
                .unwrap();
            let url = http_url(&format!("http://{host}:{}/image.jpg", address.port())).unwrap();
            assert!(network.get_asset(&url).await.is_err(), "{host}");
        }
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        let temp = tempfile::tempdir().unwrap();
        let engine = crate::Engine::open(temp.path()).await.unwrap();
        let config:NewJob=serde_json::from_value(serde_json::json!({"url":"https://example.com/","title":"CDN MIME","kind":"website","max_items":1,"max_depth":0,"delay_ms":1000,"max_resource_mb":1,"language":"eng","description":"MIME fixture","creator":"Fixture","include_assets":true})).unwrap();
        let job = engine.create_record(config, false).await.unwrap();
        let record = engine.record(&job.id).await.unwrap();
        let mut network = Network::new(&record).await.unwrap();
        network.client = Client::builder()
            .no_proxy()
            .resolve("cdn.zyrosite.com", address)
            .build()
            .unwrap();
        let error = network
            .get_asset(
                &http_url(&format!(
                    "http://cdn.zyrosite.com:{}/wrong.css",
                    address.port()
                ))
                .unwrap(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("instead of an image"));
        server.abort();
    }

    struct CdnFixture {
        active: std::sync::atomic::AtomicUsize,
        peak: std::sync::atomic::AtomicUsize,
        robots: std::sync::atomic::AtomicUsize,
        initial_group: tokio::sync::Barrier,
        replacement_started: tokio::sync::Notify,
        peak_after_limit: std::sync::atomic::AtomicUsize,
        limit_at: std::sync::Mutex<Option<Instant>>,
        first_after_limit: std::sync::Mutex<Option<Instant>>,
    }

    async fn check_cdn_batch(
        host: &str,
        own_site: bool,
        crawl_delay: bool,
        budget: bool,
        throttle: bool,
        parallel_requests: usize,
    ) {
        use axum::{
            body::{Body, Bytes},
            extract::Path,
            response::{IntoResponse, Response},
            routing::get,
            Router,
        };
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

        struct Active(Arc<CdnFixture>);
        impl Drop for Active {
            fn drop(&mut self) {
                self.0.active.fetch_sub(1, SeqCst);
            }
        }

        let test_refill = host == "cdn.arstechnica.net" && parallel_requests == 6;
        let fixture = Arc::new(CdnFixture {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            robots: AtomicUsize::new(0),
            initial_group: tokio::sync::Barrier::new(default_cdn_parallel_requests()),
            replacement_started: Default::default(),
            peak_after_limit: AtomicUsize::new(0),
            limit_at: Default::default(),
            first_after_limit: Default::default(),
        });
        let state = fixture.clone();
        let app = Router::new().route(
            "/{*path}",
            get(move |Path(path): Path<String>| {
                let state = state.clone();
                async move {
                    if path == "robots.txt" {
                        state.robots.fetch_add(1, SeqCst);
                        return if crawl_delay {
                            "User-agent: *\nCrawl-delay: 0.1"
                        } else {
                            "User-agent: *\nAllow: /"
                        }
                        .into_response();
                    }
                    let after_limit = state.limit_at.lock().unwrap().is_some();
                    if throttle
                        && !after_limit
                        && matches!(path.as_str(), "0.png" | "1.png" | "2.css")
                    {
                        state.initial_group.wait().await;
                    }
                    if throttle && path == "0.png" && !after_limit {
                        *state.limit_at.lock().unwrap() = Some(Instant::now());
                        return (
                            StatusCode::TOO_MANY_REQUESTS,
                            [("Retry-After", "3")],
                            "slow down",
                        )
                            .into_response();
                    }
                    let active = state.active.fetch_add(1, SeqCst) + 1;
                    state.peak.fetch_max(active, SeqCst);
                    if after_limit {
                        state.peak_after_limit.fetch_max(active, SeqCst);
                        state
                            .first_after_limit
                            .lock()
                            .unwrap()
                            .get_or_insert_with(Instant::now);
                    }
                    let (mime, bytes) = if path.ends_with(".css") {
                        ("text/css", "@font-face { src: url(/nested.woff2) }")
                    } else if path.ends_with(".woff2") {
                        ("font/woff2", "font bytes")
                    } else {
                        ("image/png", "image bytes")
                    };
                    if path == "nested.woff2" {
                        state.replacement_started.notify_one();
                    }
                    let wait_for_replacement = test_refill && path == "5.png";
                    let guard = Active(state);
                    // Hold the server's response body open after headers are sent,
                    // so a limit on headers alone cannot pass this test.
                    let body = Body::from_stream(futures_util::stream::once(async move {
                        if wait_for_replacement {
                            guard.0.replacement_started.notified().await;
                        }
                        let _guard = guard;
                        tokio::time::sleep(Duration::from_millis(240)).await;
                        Ok::<_, std::convert::Infallible>(Bytes::from_static(bytes.as_bytes()))
                    }));
                    Response::builder()
                        .header("Content-Type", mime)
                        .header("ETag", "\"fixture\"")
                        .body(body)
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let origin = format!("http://{host}:{}", address.port());
        let source = if own_site {
            format!("{origin}/")
        } else {
            "https://example.test/".into()
        };
        let temp = tempfile::tempdir().unwrap();
        let engine = crate::Engine::open(temp.path()).await.unwrap();
        let mut config: NewJob = serde_json::from_value(serde_json::json!({
            "url": source, "title": "CDN batch fixture", "kind": "website", "max_items": 1, "max_depth": 0,
            "delay_ms": 1000, "max_resource_mb": 1, "language": "eng", "description": "Linked asset fixture", "creator": "Test",
            "include_assets": true
        })).unwrap();
        if budget {
            config.capture.max_download_mb = Some(1);
        }
        let job = engine.create_record(config, false).await.unwrap();
        let record = engine.record(&job.id).await.unwrap();
        let mut assets = ["0.png", "1.png", "2.css", "3.png", "4.png", "5.png"]
            .map(|path| format!("{origin}/{path}"))
            .to_vec();
        if test_refill {
            assets.push(format!("{origin}/2-ordinary"));
        }
        let entry: Entry = serde_json::from_value(serde_json::json!({
            "id": "page", "url": source, "title": "Page", "depth": 0, "selected": true,
            "state": "pending", "text": "", "assets": assets
        }))
        .unwrap();
        record.change(|j| j.entries.push(entry)).await.unwrap();
        let mut network = Network::new(&record).await.unwrap();
        network.client = Client::builder()
            .no_proxy()
            .resolve(host, address)
            .build()
            .unwrap();
        tokio::time::timeout(
            Duration::from_secs(20),
            download_with_network(&record, &network, parallel_requests),
        )
        .await
        .unwrap()
        .unwrap();

        let job = record.job.lock().await;
        assert_eq!(fixture.robots.load(SeqCst), 1);
        assert_eq!(job.resources.len(), if test_refill { 8 } else { 7 });
        for resource in job.resources.values() {
            assert_eq!(resource.state, "done");
            assert_eq!(resource.etag.as_deref(), Some("\"fixture\""));
            assert_eq!(
                std::fs::metadata(record.dir.join("cache").join(&resource.path))
                    .unwrap()
                    .len(),
                resource.size
            );
        }
        assert!(job
            .resources
            .contains_key(&format!("{origin}/nested.woff2")));
        if throttle {
            let limited = fixture.limit_at.lock().unwrap().unwrap();
            let resumed = fixture.first_after_limit.lock().unwrap().unwrap();
            assert!(resumed.duration_since(limited) >= Duration::from_secs(3));
            assert_eq!(fixture.peak_after_limit.load(SeqCst), 1);
            assert!(job
                .logs
                .iter()
                .any(|log| log.message.contains("Continuing one file at a time")));
        } else {
            let expected = if is_static_cdn(&http_url(&origin).unwrap())
                && !own_site
                && !crawl_delay
                && !budget
            {
                parallel_requests
            } else {
                1
            };
            assert_eq!(
                fixture.peak.load(SeqCst),
                expected,
                "{host}, own site {own_site}, crawl delay {crawl_delay}, budget {budget}"
            );
        }
        server.abort();
    }

    #[tokio::test]
    async fn parallel_cdn_bodies_are_bounded_and_page_origins_delays_and_budgets_stay_serial() {
        tokio::join!(
            check_cdn_batch("cdn.jsdelivr.net", false, false, false, false, 1),
            check_cdn_batch("cdn.jsdelivr.net", false, false, false, false, 3),
            check_cdn_batch("cdn.arstechnica.net", false, false, false, false, 6),
            check_cdn_batch("cdn.jsdelivr.net", true, false, false, false, 3),
            check_cdn_batch("cdn.jsdelivr.net", false, true, false, false, 3),
            check_cdn_batch("cdn.jsdelivr.net", false, false, true, false, 3),
            check_cdn_batch("static.example.test", false, false, false, false, 3),
        );
    }

    #[tokio::test]
    async fn cdn_rate_limit_cancels_batch_and_recovers_serially_after_retry_after() {
        check_cdn_batch("cdn.jsdelivr.net", false, false, false, true, 3).await;
    }

    #[tokio::test]
    async fn host_outages_stop_once_and_keep_other_assets_queued_for_resume() {
        use axum::{routing::get, Router};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let hits = Arc::new(AtomicUsize::new(0));
        let count = hits.clone();
        let app = Router::new().route(
            "/robots.txt",
            get(move || {
                let count = count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        [("Retry-After", "1")],
                        "wait",
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let temp = tempfile::tempdir().unwrap();
        let engine = crate::Engine::open(temp.path()).await.unwrap();
        let source = format!("http://{address}/");
        let config:NewJob=serde_json::from_value(serde_json::json!({"url":source,"title":"Queued recovery","kind":"website","max_items":1,"max_depth":0,"delay_ms":1000,"max_resource_mb":1,"language":"eng","description":"Outage fixture","creator":"Fixture","include_assets":true})).unwrap();
        let job = engine.create_record(config, false).await.unwrap();
        let record = engine.record(&job.id).await.unwrap();
        let entry:Entry=serde_json::from_value(serde_json::json!({"id":"page","url":source,"title":"Page","depth":0,"selected":true,"state":"pending","size":null,"duration":null,"error":null,"text":"Saved page","assets":[format!("http://{address}/one.png"),format!("http://{address}/two.png"),format!("http://{address}/three.png")],"media_file":null})).unwrap();
        record.change(|job| job.entries.push(entry)).await.unwrap();
        let error = download(&record, default_cdn_parallel_requests())
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("Remaining files are still queued"));
        let job = record.job.lock().await;
        assert_eq!(
            job.resources
                .values()
                .filter(|r| r.state == "failed")
                .count(),
            1
        );
        assert_eq!(
            job.resources
                .values()
                .filter(|r| r.state == "pending")
                .count(),
            2
        );
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert!(job.retry_at.is_some());
        server.abort();
    }

    #[test]
    fn accepts_hostname_sources_without_repairing_unsafe_schemes_or_discovered_links() {
        for (input, expected) in [
            ("  CNN.com  ", "https://cnn.com/"),
            (
                "www.example.com/news?sort=latest#top",
                "https://www.example.com/news?sort=latest",
            ),
            (
                "//youtube.com/playlist?list=PL123",
                "https://youtube.com/playlist?list=PL123",
            ),
            ("localhost:8788/path", "https://localhost:8788/path"),
            ("[::1]:8788/path", "https://[::1]:8788/path"),
            ("http://example.com/path", "http://example.com/path"),
            ("bücher.de", "https://xn--bcher-kva.de/"),
        ] {
            assert_eq!(source_url(input).unwrap().as_str(), expected);
        }
        for input in [
            "",
            "javascript:alert(1)",
            "data:text/html,x",
            "mailto:a@b.com",
            "ftp://example.com",
            "user:pass@example.com",
            "https://user:pass@example.com",
            "cnn.com\n.evil.test",
        ] {
            assert!(
                source_url(input).is_err(),
                "accepted invalid source: {input}"
            );
        }
        assert!(
            http_url("cnn.com").is_err(),
            "discovered links must still resolve against their actual base URL"
        );
    }
    #[test]
    fn canonical_first_page_redirects_keep_the_same_site_path_and_security() {
        for (from, to, allowed) in [
            ("http://cnn.com/", "https://www.cnn.com/", true),
            (
                "https://www.example.com/news",
                "https://example.com/news/",
                true,
            ),
            ("https://example.com/", "http://example.com/", false),
            (
                "https://example.com/",
                "https://example.com.evil.test/",
                false,
            ),
            (
                "https://example.com/news",
                "https://www.example.com/private",
                false,
            ),
            ("http://localhost:8080/", "http://localhost:9090/", false),
        ] {
            assert_eq!(
                canonical_start_redirect(&http_url(from).unwrap(), &http_url(to).unwrap()),
                allowed
            );
        }
    }
    #[test]
    fn scope_and_canonical_urls() {
        let root = http_url("https://example.com/docs").unwrap();
        assert!(in_scope(
            &root,
            &http_url("https://example.com/docs/a?q=1#part").unwrap()
        ));
        assert!(!in_scope(
            &root,
            &http_url("https://example.com/docs-other").unwrap()
        ));
        assert!(!in_scope(
            &root,
            &http_url("https://other.com/docs").unwrap()
        ));
        assert!(http_url("file:///etc/passwd").is_err());
        assert!(http_url("https://user:pass@example.com/").is_err());
    }
    #[test]
    fn honors_retry_after() {
        assert!(retry_delay(Some("120"), 0).as_secs() >= 120);
        let future =
            httpdate::fmt_http_date(std::time::SystemTime::now() + Duration::from_secs(90));
        assert!(retry_delay(Some(&future), 0).as_secs() >= 89);
    }
    #[test]
    fn discovers_css_and_page_assets() {
        let base = http_url("https://example.com/docs/").unwrap();
        let data = inspect("<title>Guide</title><body><a href='next#x'>Next</a><link rel='stylesheet' href='/a.css'><img src='/a.png'><div style=\"background:url('/bg.png')\"></div></body>", &base);
        assert_eq!(data.title, "Guide");
        assert_eq!(data.links, vec!["https://example.com/docs/next"]);
        assert_eq!(data.assets.len(), 3);
        let urls = css_urls("@import 'a.css';a{background:url(../x.png)}", &base);
        assert_eq!(urls.len(), 2);
    }
}
