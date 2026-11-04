//! Platform browser integration. The core still owns scope, checkpoints and packaging.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::RwLock;

pub type RenderFuture = Pin<Box<dyn Future<Output = Result<RenderedPage>> + Send>>;
pub trait PageRenderer: Send + Sync {
    fn render(&self, request: RenderRequest) -> RenderFuture;
}
pub type RendererSlot = Arc<RwLock<Option<Arc<dyn PageRenderer>>>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenderRequest {
    #[serde(skip)]
    pub events: Option<tokio::sync::mpsc::UnboundedSender<RenderEvent>>,
    pub id: String,
    pub url: String,
    pub delay_ms: u64,
    pub timeout_secs: u64,
    pub max_html_bytes: u64,
    pub scopes: Vec<String>,
}
#[derive(Clone, Debug)]
pub enum RenderEvent {
    Started,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderInfo {
    pub settled: bool,
    pub resource_count: usize,
    pub known_transfer_bytes: u64,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenderedPage {
    pub url: String,
    pub html: String,
    #[serde(flatten)]
    pub info: RenderInfo,
    #[serde(default)]
    pub rate_limit: Option<RateLimit>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RateLimit {
    pub url: String,
    pub status: u16,
    pub retry_after: Option<String>,
}

pub const CAPTURE_SCRIPT: &str = include_str!("../assets/render.js");

pub async fn capture(
    record: &crate::Record,
    url: &url::Url,
    original: &str,
    delay: std::time::Duration,
) -> Result<RenderedPage> {
    let renderer = record.renderer.read().await.clone().ok_or_else(|| anyhow::anyhow!("JavaScript rendering requires an installed Zimsmith app with its browser renderer. Use static capture or import saved HTML in this build."))?;
    let config = record.job.lock().await.config.clone();
    record
        .progress(
            format!("Rendering JavaScript and lazy content: {url}"),
            0,
            None,
            "pages",
        )
        .await;
    crate::web::pace(url, delay).await;
    let mut scopes: Vec<_> = config.starting_pages().into_iter().map(|p| p.url).collect();
    if !config.contains_url(url) {
        scopes.push(url.to_string());
    }
    let (events, mut updates) = tokio::sync::mpsc::unbounded_channel();
    let request = RenderRequest {
        events: Some(events),
        id: uuid::Uuid::new_v4().to_string(),
        url: url.to_string(),
        delay_ms: delay.as_millis().min(86_400_000) as u64,
        timeout_secs: config.capture.render_timeout_secs,
        max_html_bytes: (config.max_resource_mb * 1024 * 1024).min(8 * 1024 * 1024),
        scopes,
    };
    let timeout = request.timeout_secs;
    let queued = tokio::time::Instant::now();
    let mut started = None;
    let mut future = renderer.render(request);
    let mut result = loop {
        tokio::select! {
            value = &mut future => break value?,
            Some(RenderEvent::Started) = updates.recv() => started = Some(tokio::time::Instant::now()),
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                if let Some(start) = started {
                    let elapsed = start.elapsed().as_secs();
                    anyhow::ensure!(elapsed < timeout + 15, "The page browser did not respond before its rendering deadline. Resume to retry.");
                    record.progress(format!("Rendering JavaScript · {elapsed}s elapsed"), elapsed.min(timeout), Some(timeout), "seconds").await;
                } else {
                    anyhow::ensure!(queued.elapsed().as_secs() < 300, "The page browser stayed busy for five minutes. Pause another rendered crawl and retry.");
                    record.progress("Waiting for the on-device page browser", 0, None, "pages").await;
                }
            }
        }
    };
    crate::web::cooldown(url, delay).await;
    if let Some(rate) = &result.rate_limit {
        let target = crate::web::http_url(&rate.url).unwrap_or_else(|_| url.clone());
        let wait = crate::web::retry_delay(Some(rate.retry_after.as_deref().unwrap_or("60")), 0);
        crate::web::cooldown(&target, wait).await;
        record
            .change(|j| {
                j.retry_at = Some(crate::now() + wait.as_secs() + 1);
            })
            .await?;
        anyhow::bail!("HTTP {} while rendering {target}. Backing off for {}s; Resume retries after the server cooldown.", rate.status, wait.as_secs());
    }
    anyhow::ensure!(
        result.html.len() as u64 <= (config.max_resource_mb * 1024 * 1024).min(8 * 1024 * 1024),
        "Rendered HTML exceeds its capture limit"
    );
    anyhow::ensure!(!crate::web::is_challenge(&result.html), "The browser received a verification challenge instead of page content. Retry later or import a saved page.");
    if !crate::snapshot::missing_catalogs(&scraper::Html::parse_document(original)).is_empty() {
        anyhow::ensure!(crate::snapshot::missing_catalogs(&scraper::Html::parse_document(&result.html)).is_empty(), "The JavaScript store catalog did not load before capture finished. Increase the rendering timeout or retry; no empty catalog was marked as captured.");
    }
    result.info.warnings.truncate(20);
    Ok(result)
}

impl crate::Engine {
    pub async fn set_page_renderer(&self, renderer: Arc<dyn PageRenderer>) {
        *self.renderer.write().await = Some(renderer);
    }
}
