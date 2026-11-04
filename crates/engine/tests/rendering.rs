use axum::{response::Html, routing::get, Router};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use zim_engine::{
    render::{PageRenderer, RateLimit, RenderFuture, RenderInfo, RenderRequest, RenderedPage},
    Engine, Job, NewJob, SourceKind,
};

#[derive(Clone, Copy)]
enum Mode {
    Success,
    Pending,
    RateOnce,
}
struct Renderer {
    mode: Mode,
    calls: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}
struct Guard(Arc<AtomicUsize>);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl PageRenderer for Renderer {
    fn render(&self, request: RenderRequest) -> RenderFuture {
        let mode = self.mode;
        let calls = self.calls.clone();
        let dropped = self.dropped.clone();
        Box::pin(async move {
            let call = calls.fetch_add(1, Ordering::SeqCst);
            if matches!(mode, Mode::Pending) {
                let _guard = Guard(dropped);
                return std::future::pending().await;
            }
            let rate_limit = if matches!(mode, Mode::RateOnce) && call == 0 {
                Some(RateLimit {
                    url: request.url.clone(),
                    status: 429,
                    retry_after: Some("2".into()),
                })
            } else {
                None
            };
            let html = if request.url.ends_with("/product") {
                "<title>Rendered product</title><p>Quercus specimen available offline.</p>"
            } else {
                "<title>Rendered store</title><div class='block-product-list' id='catalog'><a href='/product'>Quercus specimen</a></div><script>window.dynamic=true;</script>"
            };
            Ok(RenderedPage {
                url: request.url,
                html: html.into(),
                info: RenderInfo {
                    settled: true,
                    resource_count: 3,
                    ..Default::default()
                },
                rate_limit,
            })
        })
    }
}
fn renderer(mode: Mode) -> Arc<Renderer> {
    Arc::new(Renderer {
        mode,
        calls: Arc::new(AtomicUsize::new(0)),
        dropped: Arc::new(AtomicUsize::new(0)),
    })
}
async fn fixture() -> (String, tokio::task::JoinHandle<()>) {
    let app=Router::new().route("/robots.txt",get(||async{"User-agent: *\nDisallow: /private\n"}))
        .route("/",get(||async{Html("<title>Store shell</title><div class='block-product-list' id='catalog'></div><script>loadStore()</script>")}))
        .route("/product",get(||async{Html("<title>Product shell</title><main>Loading</main>")}))
        .route("/categories",get(||async{Html("<title>Catalog shell</title><div class='block-product-list' id='catalog'><nav><a class='category-list__link' href='/categories'>All products</a></nav><div class='skeleton-loader'>Loading</div></div><script>loadStore()</script>")}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    (
        url,
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }),
    )
}
fn config(url: String) -> NewJob {
    let mut config = NewJob {
        url,
        title: "Rendered collection".into(),
        kind: SourceKind::Website,
        max_items: 10,
        max_depth: 1,
        delay_ms: 1000,
        max_resource_mb: 8,
        language: "eng".into(),
        description: "Rendering fixture".into(),
        creator: "Fixture".into(),
        include_assets: false,
        capture: Default::default(),
    };
    config.capture.render_javascript = true;
    config
}
async fn wait(engine: &Engine, id: &str, status: &str) -> Job {
    for _ in 0..250 {
        let job = engine.get(id).await.unwrap();
        if job.status == status {
            return job;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!(
        "Expected {status}: {:?}",
        engine.get(id).await.unwrap().error
    )
}

#[tokio::test]
async fn renders_before_discovery_requires_review_and_builds_an_offline_searchable_archive() {
    let (url, server) = fixture().await;
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let renderer = renderer(Mode::Success);
    engine.set_page_renderer(renderer.clone()).await;
    let job = engine.create(config(url)).await.unwrap();
    let review = wait(&engine, &job.id, "review").await;
    assert_eq!(review.entries.len(), 2);
    assert!(review
        .entries
        .iter()
        .all(|e| e.rendered.as_ref().unwrap().settled));
    assert!(!review.can_auto_start());
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    engine
        .build(
            &job.id,
            review.entries.iter().map(|e| e.id.clone()).collect(),
        )
        .await
        .unwrap();
    let done = wait(&engine, &job.id, "completed").await;
    assert_eq!(done.capture_report.as_ref().unwrap().javascript_pages, 2);
    let archive = zimru::Archive::open(engine.artifact(&job.id).await.unwrap()).unwrap();
    let html = archive
        .get_text(&format!("{}.html", review.entries[0].id))
        .unwrap();
    assert!(html.contains("Quercus"));
    assert!(!html.contains("<script"));
    assert!(!html.contains("Store catalog not captured"));
    assert!(archive.get_text("index.html").unwrap().contains("quercus"));
    server.abort();
}

#[tokio::test]
async fn automatic_mode_renders_a_catalog_with_category_navigation_without_advanced_settings() {
    let (url, server) = fixture().await;
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let renderer = renderer(Mode::Success);
    engine.set_page_renderer(renderer.clone()).await;
    let mut config = config(format!("{url}categories"));
    config.capture.render_javascript = false;
    config.max_depth = 0;
    let job = engine.create(config).await.unwrap();
    let review = wait(&engine, &job.id, "review").await;
    assert_eq!(renderer.calls.load(Ordering::SeqCst), 1);
    assert!(review.entries[0].rendered.is_some());
    assert!(!review.can_auto_start());
    assert!(review
        .logs
        .iter()
        .any(|log| log.message.contains("automatically rendering")));
    server.abort();
}

#[tokio::test]
async fn automatic_mode_keeps_review_when_budget_or_platform_prevents_rendering() {
    for (with_renderer, static_only) in [(false, false), (true, false), (true, true)] {
        let (url, server) = fixture().await;
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::open(temp.path()).await.unwrap();
        let renderer = renderer(Mode::Success);
        if with_renderer {
            engine.set_page_renderer(renderer.clone()).await;
        }
        let mut config = config(url);
        config.capture.render_javascript = false;
        config.capture.auto_render_javascript = !static_only;
        config.max_depth = 0;
        if with_renderer && !static_only {
            config.capture.max_download_mb = Some(1);
        }
        let job = engine.create(config).await.unwrap();
        let review = wait(&engine, &job.id, "review").await;
        assert_eq!(renderer.calls.load(Ordering::SeqCst), 0);
        assert!(review.scan_truncated);
        assert!(review.javascript_needed);
        assert!(review
            .logs
            .iter()
            .any(|log| log.message.contains("Dynamic content detected")));
        assert!(!review.can_auto_start());
        server.abort();
    }
}

#[tokio::test]
async fn pausing_drops_the_renderer_and_resume_keeps_the_scan_checkpoint() {
    let (url, server) = fixture().await;
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let blocked = renderer(Mode::Pending);
    engine.set_page_renderer(blocked.clone()).await;
    let job = engine.create(config(url)).await.unwrap();
    for _ in 0..100 {
        if blocked.calls.load(Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(blocked.calls.load(Ordering::SeqCst), 1);
    engine.pause(&job.id).await.unwrap();
    let paused = wait(&engine, &job.id, "paused").await;
    assert!(paused.entries.is_empty());
    assert_eq!(paused.queue.len(), 1);
    assert_eq!(paused.clone().view().queued_pages, 1);
    assert_eq!(blocked.dropped.load(Ordering::SeqCst), 1);
    engine.set_page_renderer(renderer(Mode::Success)).await;
    engine.resume(&job.id).await.unwrap();
    assert_eq!(wait(&engine, &job.id, "review").await.entries.len(), 2);
    server.abort();
}

#[tokio::test]
async fn rendered_rate_limits_persist_cooldowns_and_retry_without_publishing_empty_content() {
    let (url, server) = fixture().await;
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let renderer = renderer(Mode::RateOnce);
    engine.set_page_renderer(renderer.clone()).await;
    let job = engine.create(config(url)).await.unwrap();
    let failed = wait(&engine, &job.id, "failed").await;
    assert!(failed.retry_at.is_some());
    assert!(failed.entries[0].error.as_ref().unwrap().contains("429"));
    assert!(failed.entries[0].rendered.is_none());
    assert!(failed.output.is_none());
    let before = failed.requests;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    engine.resume(&job.id).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(engine.get(&job.id).await.unwrap().requests, before);
    assert_eq!(wait(&engine, &job.id, "review").await.entries.len(), 2);
    server.abort();
}

#[tokio::test]
async fn unavailable_renderers_and_strict_budgets_fail_before_starting_a_job() {
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let mut cfg = config("https://example.test/".into());
    assert!(engine
        .create(cfg.clone())
        .await
        .unwrap_err()
        .to_string()
        .contains("unavailable"));
    cfg.capture.max_download_mb = Some(50);
    engine.set_page_renderer(renderer(Mode::Success)).await;
    assert!(engine
        .create(cfg)
        .await
        .unwrap_err()
        .to_string()
        .contains("strict download budget"));
    assert!(engine.list().await.is_empty());
}
