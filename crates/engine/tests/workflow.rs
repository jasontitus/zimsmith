use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use zim_engine::{ArchiveTemplate, Engine, Job, NewJob, Settings, SourceKind};

fn config(url: String) -> NewJob {
    NewJob {
        url,
        title: "Local test collection".into(),
        kind: SourceKind::Website,
        max_items: 5,
        max_depth: 2,
        delay_ms: 1000,
        max_resource_mb: 2,
        language: "eng".into(),
        description: "Offline fixture".into(),
        creator: "Fixture".into(),
        include_assets: true,
        capture: Default::default(),
    }
}

#[tokio::test]
async fn original_website_starts_at_the_root_keeps_contents_and_rebuilds_offline() {
    let app = Router::new()
        .route("/robots.txt", get(|| async { "User-agent: *\nAllow: /\n" }))
        .route("/", get(|| async { Html("<html><head><title>Original home</title></head><body><h1>Original home</h1><a href='/child'>Visit child</a><div class='product-carousel__image-wrapper'><template><img class='ecommerce-product-image ecommerce-product-image--loading' src='/photo.svg'></template></div></body></html>") }))
        .route("/child", get(|| async { Html("<html><title>Child page</title><body><a href='/'>Home</a></body></html>") }))
        .route("/photo.svg", get(|| async { ([("Content-Type", "image/svg+xml")], "<svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'/>") }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    engine
        .update_settings(Settings {
            archive_template: ArchiveTemplate::Website,
            ..Settings::default()
        })
        .await
        .unwrap();
    let started = engine.create(config(url.clone())).await.unwrap();
    let job = wait(&engine, &started.id, "completed").await;
    let root = job.entries.iter().find(|e| e.url == url).unwrap();
    let child = job
        .entries
        .iter()
        .find(|e| e.url.ends_with("/child"))
        .unwrap();
    let archive = zimru::Archive::open(job.output.as_ref().unwrap()).unwrap();
    assert_eq!(archive.main_path().unwrap(), format!("{}.html", root.id));
    let html = archive.get_text(&archive.main_path().unwrap()).unwrap();
    assert!(html.contains(&format!("{}.html", child.id)));
    assert!(!html.contains("zim-archive-nav"));
    assert!(!html.contains("ecommerce-product-image--loading"));
    assert!(!html.contains("<template"));
    assert!(html.contains("zim-static-product-image"));
    assert!(archive
        .get_text("index.html")
        .unwrap()
        .contains("search.js"));
    let requests = job.requests;
    drop(archive);
    server.abort();
    engine
        .rebuild(&job.id, ArchiveTemplate::Website)
        .await
        .unwrap();
    let rebuilt = wait(&engine, &job.id, "completed").await;
    assert_eq!(rebuilt.requests, requests);
    assert!(rebuilt.validation.unwrap().checksum_verified);
}

#[tokio::test]
async fn sitemap_lazy_images_and_embedded_video_review_work_together() {
    let images = Arc::new(AtomicUsize::new(0));
    let sitemap_hits = Arc::new(AtomicUsize::new(0));
    let maps = sitemap_hits.clone();
    let app = Router::new()
        .route("/robots.txt", get(|| async { "User-agent: *\nDisallow: /private\nSitemap: /catalog.xml\n" }))
        .route("/", get(|| async { Html(r#"<title>Home</title><iframe title="Talk" src="https://www.youtube-nocookie.com/embed/abcdefghijk"></iframe><iframe src="https://www.youtube.com/embed/abcdefghijk"></iframe><img src="data:image/gif;base64,AA" data-original="/photo.svg">"#) }))
        .route("/sitemap.xml", get(move || { let maps=maps.clone(); async move { maps.fetch_add(1,Ordering::SeqCst); ([("Content-Type","application/xml")], "<sitemapindex><sitemap><loc>/catalog.xml</loc></sitemap><sitemap><loc>https://outside.test/sitemap.xml</loc></sitemap><sitemap><loc>/jump.xml</loc></sitemap></sitemapindex>") } }))
        .route("/jump.xml", get(|| async { axum::response::Redirect::temporary("https://outside.test/redirected-map.xml") }))
        .route("/catalog.xml", get(|| async { ([("Content-Type","application/xml")], "<urlset><url><loc>/orphan</loc></url><url><loc>/private</loc></url><url><loc>https://outside.test/page</loc></url></urlset>") }))
        .route("/orphan", get(|| async { Html("<title>Unlinked page</title><p>Discovered only through the sitemap.</p>") }))
        .route("/private", get(|| async { "should never fetch" }))
        .route("/photo.svg", get(|State(images): State<Arc<AtomicUsize>>| async move { images.fetch_add(1,Ordering::SeqCst); ([("Content-Type","image/svg+xml")],"<svg xmlns='http://www.w3.org/2000/svg'/>") }))
        .with_state(images.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let mut cfg = config(url.clone());
    cfg.max_items = 10;
    cfg.capture.discover_sitemaps = true;
    let job = engine.create(cfg).await.unwrap();
    let review = wait(&engine, &job.id, "review").await;
    assert_eq!(
        images.load(Ordering::SeqCst),
        0,
        "no image payload before approval"
    );
    assert_eq!(sitemap_hits.load(Ordering::SeqCst), 1);
    assert!(review.logs.iter().any(|l| l
        .message
        .contains("Cross-origin sitemap redirect ignored before requesting")));
    assert!(review.entries.iter().any(|e| e.title == "Unlinked page"));
    let video = review
        .entries
        .iter()
        .filter(|e| !e.embedded_in.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(video.len(), 1, "duplicate embeds share one download");
    assert_eq!(video[0].embedded_in, [url.clone()]);
    assert_eq!(video[0].size, None);
    assert!(
        !review.can_auto_start(),
        "unknown-size embedded videos require review"
    );
    assert!(review
        .entries
        .iter()
        .any(|e| e.url.ends_with("/private") && e.state == "scan_failed"));
    let selected = review
        .entries
        .iter()
        .filter(|e| e.state != "scan_failed" && e.embedded_in.is_empty())
        .map(|e| e.id.clone())
        .collect();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    engine.build(&job.id, selected).await.unwrap();
    wait(&engine, &job.id, "completed").await;
    assert_eq!(images.load(Ordering::SeqCst), 1);
    let archive = zimru::Archive::open(engine.artifact(&job.id).await.unwrap()).unwrap();
    let home = review.entries.iter().find(|e| e.url == url).unwrap();
    let html = archive.get_text(&format!("{}.html", home.id)).unwrap();
    assert!(html.contains("Watch video online"));
    assert!(!html.contains("<iframe"));
    assert!(html.contains(".svg"));
    assert!(!html.contains("src=\"data:image/gif"));
    server.abort();
}

#[tokio::test]
async fn cached_embedded_video_is_packaged_inline_with_subtitles_and_no_network() {
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = saved_attempt(temp.path(), "embedded-fixture", "completed").await;
    fixture.resources.clear();
    fixture.phase = "package".into();
    fixture.entries[0].state = "done".into();
    fixture.entries[0].assets.clear();
    let mut video = fixture.entries[0].clone();
    video.id = "embedded-video".into();
    video.url = "https://www.youtube.com/watch?v=abcdefghijk".into();
    video.title = "Offline talk".into();
    video.embedded_in = vec![fixture.config.url.clone()];
    video.media_file = Some("movie.mp4".into());
    video.media_audio = Some("audio.m4a".into());
    video.thumbnail_url = None;
    video.subtitles = vec![zim_engine::captions::SavedSubtitle {
        file: "captions.vtt".into(),
        language: "en".into(),
        label: "English".into(),
        auto_generated: false,
    }];
    fixture.entries.push(video);
    let dir = temp.path().join("embedded-fixture");
    tokio::fs::create_dir_all(dir.join("media")).await.unwrap();
    tokio::fs::write(
        dir.join("media/movie.mp4"),
        include_bytes!("fixtures/sync-bframes-video.mp4"),
    )
    .await
    .unwrap();
    tokio::fs::write(
        dir.join("media/audio.m4a"),
        include_bytes!("fixtures/sync-audio.m4a"),
    )
    .await
    .unwrap();
    tokio::fs::write(
        dir.join("media/captions.vtt"),
        "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nSaved words\n",
    )
    .await
    .unwrap();
    tokio::fs::write(dir.join("cache/home.html"),r#"<title>Home</title><iframe src="https://www.youtube.com/embed/abcdefghijk"></iframe><div class="video" id="card"><img src="https://i.ytimg.com/vi/abcdefghijk/hqdefault.jpg"><button class="video__play--youtube"></button></div>"#).await.unwrap();
    tokio::fs::write(dir.join("job.json"), serde_json::to_vec(&fixture).unwrap())
        .await
        .unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    engine
        .rebuild("embedded-fixture", ArchiveTemplate::Atlas)
        .await
        .unwrap();
    let done = wait(&engine, "embedded-fixture", "completed").await;
    assert_eq!(done.requests, fixture.requests);
    let archive = zimru::Archive::open(engine.artifact("embedded-fixture").await.unwrap()).unwrap();
    let page = archive.get_text("home.html").unwrap();
    assert_eq!(page.matches("<video ").count(), 2);
    assert!(page.contains("src=\"embedded-video-combined.mp4\""));
    assert!(page.contains("src=\"captions.vtt\""));
    assert!(!page.contains("<iframe") && !page.contains("Watch video online"));
    assert!(archive
        .get_entry_by_path("embedded-video-combined.mp4")
        .is_ok());
    assert!(archive
        .get_text("captions.vtt")
        .unwrap()
        .contains("Saved words"));
    assert!(!done
        .capture_report
        .unwrap()
        .warnings
        .iter()
        .any(|s| s.contains("movies were not downloaded")));
    if let Some(destination) = std::env::var_os("ZIMSTUDIO_EMBED_OUTPUT_DIR") {
        tokio::fs::create_dir_all(&destination).await.unwrap();
        let mut files = tokio::fs::read_dir(dir.join("content")).await.unwrap();
        while let Some(file) = files.next_entry().await.unwrap() {
            tokio::fs::copy(
                file.path(),
                std::path::Path::new(&destination).join(file.file_name()),
            )
            .await
            .unwrap();
        }
    }
}

#[tokio::test]
async fn available_files_recovery_omits_an_unfinished_embed_and_keeps_the_page() {
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = saved_attempt(temp.path(), "partial-embed", "failed").await;
    fixture.resources.clear();
    fixture.entries[0].assets.clear();
    let mut video = fixture.entries[0].clone();
    video.id = "unfinished-video".into();
    video.url = "https://www.youtube.com/watch?v=abcdefghijk".into();
    video.embedded_in = vec![fixture.config.url.clone()];
    video.state = "failed".into();
    fixture.entries.push(video);
    let dir = temp.path().join("partial-embed");
    tokio::fs::write(dir.join("cache/home.html"),r#"<title>Home</title><p>Saved article</p><iframe src="https://www.youtube.com/embed/abcdefghijk"></iframe>"#).await.unwrap();
    tokio::fs::write(dir.join("job.json"), serde_json::to_vec(&fixture).unwrap())
        .await
        .unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    engine.build_available(&fixture.id).await.unwrap();
    let done = wait(&engine, &fixture.id, "completed").await;
    assert_eq!(done.requests, fixture.requests);
    assert!(!done.entries[1].selected);
    let archive = zimru::Archive::open(engine.artifact(&fixture.id).await.unwrap()).unwrap();
    assert!(archive
        .get_text("home.html")
        .unwrap()
        .contains("Saved article"));
    assert!(archive
        .get_text("home.html")
        .unwrap()
        .contains("Video not saved"));
    assert!(done
        .capture_report
        .unwrap()
        .warnings
        .iter()
        .any(|s| s.contains("movies were not downloaded")));
}
async fn wait(engine: &Engine, id: &str, status: &str) -> Job {
    tokio::time::timeout(std::time::Duration::from_secs(45), async {
        loop {
            let job = engine.get(id).await.unwrap();
            if job.status == status {
                return job;
            }
            assert_ne!(job.status, "failed", "{:?}", job.error);
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("job did not reach expected state")
}

#[tokio::test]
async fn scans_requires_confirmation_resumes_and_builds_readable_zim() {
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/robots.txt", get(|| async { "User-agent: *\nDisallow: /private\n" }))
        .route("/", get(|| async { Html("<html><head><title>Home</title><link rel='stylesheet' href='/style.css'></head><body>Botany field notes<a href='/guide'>Guide</a><a href='/private'>Private</a><img src='/leaf.svg'></body></html>") }))
        .route("/guide", get(|| async { Html("<html><head><title>Field guide</title></head><body>Molluscs and animals<a href='/'>Home</a></body></html>") }))
        .route("/private", get(|| async { "robots-disallowed page" }))
        .route("/style.css", get(|| async { ([("Content-Type", "text/css")], "body{background:url('/leaf.svg')}") }))
        .route("/leaf.svg", get(|State(hits): State<Arc<AtomicUsize>>| async move { hits.fetch_add(1, Ordering::SeqCst); ([("Content-Type", "image/svg+xml")], "<svg xmlns='http://www.w3.org/2000/svg'/>") }))
        .with_state(hits.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let job = engine.create(config(url.clone())).await.unwrap();
    assert_eq!(job.auto_start_item_limit, Some(100));
    engine.pause(&job.id).await.unwrap();
    wait(&engine, &job.id, "paused").await;
    engine.resume(&job.id).await.unwrap();
    let review = wait(&engine, &job.id, "review").await;
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "assets downloaded before confirmation"
    );
    assert_eq!(review.entries.len(), 3);
    assert!(review
        .entries
        .iter()
        .any(|e| e.state == "scan_failed" && e.error.as_ref().unwrap().contains("robots")));
    let selected = review
        .entries
        .iter()
        .filter(|e| e.state != "scan_failed")
        .map(|e| e.id.clone())
        .collect();
    // Wait for runner bookkeeping to finish before issuing the next command.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    engine.build(&job.id, selected).await.unwrap();
    let done = wait(&engine, &job.id, "completed").await;
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "duplicate assets should be deduplicated"
    );
    assert!(done.output_bytes.unwrap() > 100);
    let archive = zimru::Archive::open(engine.artifact(&job.id).await.unwrap()).unwrap();
    let index = archive.get_text("index.html").unwrap();
    assert!(index.contains("Field guide"));
    assert!(index.contains("molluscs") && index.contains("catalog"));
    let home = review.entries.iter().find(|e| e.url == url).unwrap();
    let html = archive.get_text(&format!("{}.html", home.id)).unwrap();
    assert!(!html.contains("src='/leaf.svg'"));
    assert!(archive.has_checksum());
    #[cfg(feature = "kiwix-index")]
    for name in ["fulltext/xapian", "title/xapian"] {
        let entry = archive.entry_by_ns_path(b'X', name).unwrap();
        assert!(entry.get_item(false).unwrap().bytes().unwrap().len() > 100);
    }
    let recovered = Engine::open(temp.path()).await.unwrap();
    assert_eq!(recovered.get(&job.id).await.unwrap().status, "completed");
    server.abort();
}

#[tokio::test]
async fn retries_rate_limits_and_recovers_persisted_scan() {
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/robots.txt", get(|| async { "User-agent: *\nAllow: /" }))
        .route(
            "/",
            get(|State(hits): State<Arc<AtomicUsize>>| async move {
                if hits.fetch_add(1, Ordering::SeqCst) == 0 {
                    (
                        StatusCode::TOO_MANY_REQUESTS,
                        [("Retry-After", "2")],
                        "slow down",
                    )
                        .into_response()
                } else {
                    Html("<title>Recovered</title><body>Welcome</body>").into_response()
                }
            }),
        )
        .with_state(hits.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    engine
        .update_settings(Settings {
            auto_start_small_jobs: false,
            ..Settings::default()
        })
        .await
        .unwrap();
    let job = engine.create(config(url)).await.unwrap();
    let review = wait(&engine, &job.id, "review").await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert!(review.logs.iter().any(|l| l.message.contains("429")));
    // Simulate the manifest left by an interrupted process, without two engines
    // concurrently operating on the same job directory.
    let record = engine.record(&job.id).await.unwrap();
    record
        .change(|j| j.status = "scanning".into())
        .await
        .unwrap();
    drop(record);
    drop(engine);
    let recovered = Engine::open(temp.path()).await.unwrap();
    assert_eq!(recovered.get(&job.id).await.unwrap().status, "paused");
    server.abort();
}

#[tokio::test]
async fn automatically_builds_small_jobs_and_keeps_larger_or_partial_scans_for_review() {
    let assets = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/robots.txt", get(|| async { "User-agent: *\nAllow: /" }))
        .route("/", get(|| async { Html("<title>Home</title><body>Notes<a href='/guide'>Guide</a><img src='/image.svg'></body>") }))
        .route("/guide", get(|| async { Html("<title>Guide</title><body>Small offline collection</body>") }))
        .route("/image.svg", get(|State(assets): State<Arc<AtomicUsize>>| async move {
            assets.fetch_add(1, Ordering::SeqCst);
            ([("Content-Type", "image/svg+xml")], "<svg xmlns='http://www.w3.org/2000/svg'/>")
        })).with_state(assets.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    assert_eq!(engine.settings().await, Settings::default());
    engine
        .update_settings(Settings {
            auto_start_item_limit: 2,
            archive_template: ArchiveTemplate::FieldNotes,
            ..Settings::default()
        })
        .await
        .unwrap();
    let small = engine.create(config(url.clone())).await.unwrap();
    // New settings do not alter the rule saved on an already-created job.
    engine
        .update_settings(Settings {
            auto_start_item_limit: 1,
            archive_template: ArchiveTemplate::Midnight,
            ..Settings::default()
        })
        .await
        .unwrap();
    let complete = wait(&engine, &small.id, "completed").await;
    assert_eq!(complete.archive_template, ArchiveTemplate::FieldNotes);
    let archive = zimru::Archive::open(engine.artifact(&small.id).await.unwrap()).unwrap();
    assert!(archive
        .get_text("index.html")
        .unwrap()
        .contains("data-zim-template=\"field_notes\""));
    assert!(archive
        .get_bytes("archive-sans.woff2")
        .unwrap()
        .starts_with(b"wOF2"));
    assert!(archive
        .get_bytes("archive-display.woff2")
        .unwrap()
        .starts_with(b"wOF2"));
    assert!(archive
        .get_text("font-licenses.txt")
        .unwrap()
        .contains("SIL OPEN FONT LICENSE"));
    assert_eq!(complete.entries.len(), 2);
    assert!(complete
        .logs
        .iter()
        .any(|log| log.message.contains("Automatically approved 2 items")));
    assert_eq!(assets.load(Ordering::SeqCst), 1);
    let large = engine.create(config(url.clone())).await.unwrap();
    let review = wait(&engine, &large.id, "review").await;
    assert_eq!(review.entries.len(), 2);
    assert_eq!(
        assets.load(Ordering::SeqCst),
        1,
        "larger jobs must not download assets automatically"
    );
    engine.update_settings(Settings::default()).await.unwrap();
    let mut limited = config(url.clone());
    limited.max_items = 1;
    let partial = engine.create(limited).await.unwrap();
    let review = wait(&engine, &partial.id, "review").await;
    assert!(review.scan_truncated);
    assert_eq!(
        assets.load(Ordering::SeqCst),
        1,
        "truncated scans must not start automatically"
    );
    let saved = Settings {
        auto_start_small_jobs: false,
        auto_start_item_limit: 42,
        ..Settings::default()
    };
    engine.update_settings(saved.clone()).await.unwrap();
    let reopened = Engine::open(temp.path()).await.unwrap();
    assert_eq!(reopened.settings().await, saved);
    let manual = reopened.create(config(url)).await.unwrap();
    assert_eq!(manual.auto_start_item_limit, None);
    wait(&reopened, &manual.id, "review").await;
    assert_eq!(assets.load(Ordering::SeqCst), 1);
    assert!(engine
        .update_settings(Settings {
            auto_start_item_limit: 0,
            ..saved.clone()
        })
        .await
        .is_err());
    assert_eq!(engine.settings().await, saved);
    server.abort();
}

#[test]
fn detects_youtube_and_direct_media_without_matching_lookalike_hosts() {
    for url in [
        "https://youtube.com/playlist?list=PLalEW8Rh_rGk&si=B0TFoy5VtWkOP-LI",
        "https://www.youtube.com/@channel",
        "https://m.youtube.com/results?search_query=nature",
        "https://music.youtube.com/playlist?list=example",
        "https://youtu.be/example?si=shared",
        "https://example.com/clip.MP4?token=value",
    ] {
        let mut source = config(url.into());
        assert!(source.detect_media_source(), "{url}");
        assert_eq!(source.kind, SourceKind::Media);
        assert!(
            !source.detect_media_source(),
            "detection must be idempotent"
        );
    }
    for url in [
        "https://example.com/",
        "https://youtube.com.example.org/playlist?list=example",
        "https://example.com/?url=https://youtube.com/playlist",
        "https://example.com/page.mp4.html",
    ] {
        let mut source = config(url.into());
        assert!(!source.detect_media_source(), "{url}");
        assert_eq!(source.kind, SourceKind::Website);
    }
}

#[tokio::test]
async fn routes_media_and_repairs_old_html_scan_on_resume() {
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/clip.mp4",
            get(|State(hits): State<Arc<AtomicUsize>>| async move {
                hits.fetch_add(1, Ordering::SeqCst);
                ([("content-type", "video/mp4")], "fixture")
            }),
        )
        .with_state(hits.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/clip.mp4", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    engine
        .update_settings(Settings {
            auto_start_small_jobs: false,
            ..Settings::default()
        })
        .await
        .unwrap();
    let created = engine.create(config(url.clone())).await.unwrap();
    assert_eq!(created.config.kind, SourceKind::Media);
    wait(&engine, &created.id, "review").await;
    let record = engine.record(&created.id).await.unwrap();
    while record.running.load(Ordering::SeqCst) {
        tokio::task::yield_now().await;
    }
    record
        .change(|j| {
            // Reproduce a checkpoint from the previous iOS build's HTML crawler.
            j.config.kind = SourceKind::Website;
            j.status = "failed".into();
            j.error = Some("No accessible HTML pages found".into());
            j.entries[0].state = "scan_failed".into();
            j.entries[0].selected = false;
            j.entries[0].error = Some("HTML scan failed".into());
            j.visited.push(url.clone());
            j.scan_truncated = true;
            j.retry_at = Some(u64::MAX);
        })
        .await
        .unwrap();
    engine.resume(&created.id).await.unwrap();
    let review = wait(&engine, &created.id, "review").await;
    assert_eq!(review.config.kind, SourceKind::Media);
    assert_eq!(review.entries.len(), 1);
    assert_eq!(review.entries[0].state, "pending");
    assert!(review.entries[0].selected);
    assert!(review.entries[0].error.is_none());
    assert!(review.visited.is_empty());
    assert!(review.queue.is_empty());
    assert!(!review.scan_truncated);
    assert!(review.retry_at.is_none());
    assert!(review.error.is_none());
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    server.abort();
}

#[tokio::test]
#[ignore = "Live YouTube metadata check; no media is downloaded"]
async fn scans_youtube_playlist_natively() {
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    engine
        .update_settings(Settings {
            auto_start_small_jobs: false,
            ..Settings::default()
        })
        .await
        .unwrap();
    let mut source =
        config("https://youtube.com/playlist?list=PLalEW8Rh_rGk&si=B0TFoy5VtWkOP-LI".into());
    source.max_items = 20;
    let job = engine.create(source).await.unwrap();
    assert_eq!(job.config.kind, SourceKind::Media);
    let review = wait(&engine, &job.id, "review").await;
    for log in &review.logs {
        println!("{}: {}", log.level, log.message);
    }
    assert!(!review.entries.is_empty());
    assert!(review.entries.len() <= 20);
    assert!(review.entries.iter().all(|e| e.media_file.is_none()));
    assert!(
        !review
            .logs
            .iter()
            .any(|l| l.message.contains("Native extractor failed")),
        "Native scan failed and desktop yt-dlp was used"
    );
    for entry in &review.entries {
        println!("{}: {}", entry.url, entry.title);
    }
}

#[tokio::test]
#[ignore = "Live YouTube stream/CDN check; reads only a small media prefix"]
async fn resolves_youtube_media_streams_natively() {
    use rustypipe::client::{ClientType, RustyPipe};
    let temp = tempfile::tempdir().unwrap();
    let rp = RustyPipe::builder()
        .storage_dir(temp.path())
        .no_reporter()
        .build()
        .unwrap();
    let player = rp
        .query()
        .player_from_clients(
            "O6VZaBM2Zr4",
            &[ClientType::VisionOs, ClientType::Ios, ClientType::Android],
        )
        .await
        .unwrap();
    println!(
        "Stream metadata: {} combined, {} video, {} audio",
        player.video_streams.len(),
        player.video_only_streams.len(),
        player.audio_streams.len()
    );
    assert!(
        !player.video_streams.is_empty()
            || (!player.video_only_streams.is_empty() && !player.audio_streams.is_empty())
    );
    let combined = player
        .video_streams
        .iter()
        .filter(|s| s.drm_track_type.is_none())
        .max_by_key(|s| (s.height <= 720, s.height));
    let video = combined
        .or_else(|| {
            player
                .video_only_streams
                .iter()
                .filter(|s| s.drm_track_type.is_none() && s.mime.contains("mp4"))
                .max_by_key(|s| (s.height <= 720, s.height))
        })
        .expect("no supported video stream");
    let mut streams = vec![("video", video.url.as_str())];
    if combined.is_none() {
        let audio = player
            .audio_streams
            .iter()
            .filter(|s| s.drm_track_type.is_none())
            .max_by_key(|s| (s.mime.contains("mp4"), s.bitrate))
            .expect("no supported audio stream");
        streams.push(("audio", audio.url.as_str()));
    }
    for (kind, url) in streams {
        let mut response = reqwest::Client::new()
            .get(url)
            .header("Range", "bytes=0-1023")
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_success(),
            "{kind} CDN returned {}",
            response.status()
        );
        let prefix = response
            .chunk()
            .await
            .unwrap()
            .expect("empty media response");
        assert!(!prefix.is_empty());
        println!(
            "{kind} stream is accessible; read {} prefix bytes",
            prefix.len()
        );
        // Drop the response immediately even if the server ignores Range.
    }
}

#[tokio::test]
#[ignore = "Live YouTube video/channel/search metadata check; no media is downloaded"]
async fn scans_youtube_video_channel_and_search_natively() {
    for url in [
        "https://www.youtube.com/watch?v=O6VZaBM2Zr4",
        "https://www.youtube.com/@SaturdayNightLive",
        "https://www.youtube.com/results?search_query=kiwix",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::open(temp.path()).await.unwrap();
        engine
            .update_settings(Settings {
                auto_start_small_jobs: false,
                ..Settings::default()
            })
            .await
            .unwrap();
        let mut source = config(url.into());
        source.max_items = 3;
        let job = engine.create(source).await.unwrap();
        let review = wait(&engine, &job.id, "review").await;
        assert!(!review.entries.is_empty(), "{url}");
        assert!(review.entries.iter().all(|e| e.media_file.is_none()));
        assert!(
            !review
                .logs
                .iter()
                .any(|l| l.message.contains("Native extractor failed")),
            "Native scan fell back to yt-dlp for {url}: {:?}",
            review.logs
        );
        println!("Native scan found {} items: {url}", review.entries.len());
    }
}

#[tokio::test]
async fn rebuilds_offline_without_redownloading_and_preserves_previous_zim_on_failure() {
    let app = Router::new()
        .route("/robots.txt", get(|| async { "User-agent: *\nAllow: /" }))
        .route("/", get(|| async { Html("<html><head><title>Saved collection</title></head><body>Offline botany<img src='/leaf.svg'></body></html>") }))
        .route("/leaf.svg", get(|| async { ([("Content-Type", "image/svg+xml")], "<svg xmlns='http://www.w3.org/2000/svg'/>") }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let job = engine.create(config(url)).await.unwrap();
    let original = wait(&engine, &job.id, "completed").await;
    assert_eq!(original.archive_revision, 0);
    // Rebuilds must succeed with the origin completely offline.
    server.abort();
    let dir = temp.path().join(&job.id);
    let saved_copy = temp.path().join("previously-saved.zim");
    tokio::fs::copy(dir.join("archive.zim"), &saved_copy)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    engine
        .rebuild(&job.id, ArchiveTemplate::Midnight)
        .await
        .unwrap();
    let rebuilt = wait(&engine, &job.id, "completed").await;
    assert_eq!(rebuilt.requests, original.requests);
    assert_eq!(rebuilt.bytes, original.bytes);
    assert_eq!(rebuilt.archive_revision, 1);
    assert!(zimru::Archive::open(dir.join("archive.zim"))
        .unwrap()
        .get_text("index.html")
        .unwrap()
        .contains("data-zim-template=\"midnight\""));
    assert!(zimru::Archive::open(saved_copy)
        .unwrap()
        .get_text("index.html")
        .unwrap()
        .contains("data-zim-template=\"atlas\""));
    let good_zim = tokio::fs::read(dir.join("archive.zim")).await.unwrap();
    let resource = rebuilt
        .resources
        .values()
        .find(|r| r.state == "done")
        .unwrap();
    let asset = dir.join("cache").join(&resource.path);
    let asset_bytes = tokio::fs::read(&asset).await.unwrap();
    tokio::fs::remove_file(&asset).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    engine
        .rebuild(&job.id, ArchiveTemplate::FieldNotes)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while engine.get(&job.id).await.unwrap().status != "failed" {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        tokio::fs::read(dir.join("archive.zim")).await.unwrap(),
        good_zim,
        "failed rebuild must keep the old archive"
    );
    tokio::fs::write(asset, asset_bytes).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    engine.resume(&job.id).await.unwrap();
    let recovered = wait(&engine, &job.id, "completed").await;
    assert_eq!(recovered.requests, original.requests);
    assert_eq!(recovered.archive_template, ArchiveTemplate::FieldNotes);
    assert_eq!(recovered.archive_revision, 2);
    assert!(zimru::Archive::open(dir.join("archive.zim"))
        .unwrap()
        .get_text("index.html")
        .unwrap()
        .contains("data-zim-template=\"field_notes\""));
    // A missing main download is detected before altering the completed job.
    tokio::fs::remove_file(
        dir.join("cache")
            .join(format!("{}.html", recovered.entries[0].id)),
    )
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(engine
        .rebuild(&job.id, ArchiveTemplate::Atlas)
        .await
        .unwrap_err()
        .to_string()
        .contains("saved content"));
    assert_eq!(engine.get(&job.id).await.unwrap().status, "completed");
    assert_eq!(engine.get(&job.id).await.unwrap().archive_revision, 2);
}

#[tokio::test]
async fn editing_a_failed_crawl_restarts_the_source_without_losing_saved_files() {
    let app = Router::new()
        .route("/robots.txt", get(|| async { "User-agent: *\nAllow: /" }))
        .route("/missing", get(|| async { StatusCode::NOT_FOUND }))
        .route("/fixed", get(|| async { (StatusCode::MOVED_PERMANENTLY, [("Location", "/fixed/")], "Moved") }))
        .route("/fixed/", get(|| async { Html("<html><title>Correct source</title><body>A good page.<a href='/fixed/'>Self</a></body></html>") }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let created = engine
        .create(config(format!("{origin}/missing")))
        .await
        .unwrap();
    let failed = wait(&engine, &created.id, "failed").await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let directory = temp.path().join(&created.id);
    tokio::fs::create_dir_all(directory.join("media"))
        .await
        .unwrap();
    tokio::fs::write(directory.join("media/saved-file"), b"saved download")
        .await
        .unwrap();
    for cursor in ["media-cursor.json", "youtube-playlist-page.json"] {
        tokio::fs::write(directory.join(cursor), b"old cursor")
            .await
            .unwrap();
    }
    let mut corrected = failed.config.clone();
    corrected.url = "javascript:alert(1)".into();
    assert!(engine
        .edit_retry(&created.id, corrected.clone())
        .await
        .is_err());
    assert_eq!(
        engine.get(&created.id).await.unwrap().config.url,
        failed.config.url
    );
    corrected.url = format!(" {origin}/fixed ");
    corrected.title = "Edited collection".into();
    corrected.max_depth = 1;
    corrected.max_items = 3;
    engine.edit_retry(&created.id, corrected).await.unwrap();
    let complete = wait(&engine, &created.id, "completed").await;
    assert_eq!(complete.id, failed.id);
    assert_eq!(complete.config.url, format!("{origin}/fixed/"));
    assert_eq!(complete.config.title, "Edited collection");
    assert_eq!(complete.config.max_items, 3);
    assert_eq!(complete.config.max_depth, 1);
    assert_eq!(complete.entries.len(), 1);
    assert_eq!(complete.entries[0].title, "Correct source");
    assert_eq!(
        tokio::fs::read(directory.join("media/saved-file"))
            .await
            .unwrap(),
        b"saved download"
    );
    assert!(!directory.join("media-cursor.json").exists());
    assert!(!directory.join("youtube-playlist-page.json").exists());
    let mut history = tokio::fs::read_dir(directory.join("edit-history"))
        .await
        .unwrap();
    let previous = history.next_entry().await.unwrap().unwrap();
    let old: Job =
        serde_json::from_slice(&tokio::fs::read(previous.path()).await.unwrap()).unwrap();
    assert_eq!(old.config.url, failed.config.url);
    assert_eq!(old.status, "failed");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        engine
            .edit_retry(&created.id, complete.config)
            .await
            .is_err(),
        "completed collections cannot be edited in place"
    );
    server.abort();
}

#[tokio::test]
async fn editing_metadata_retries_packaging_offline_without_restarting_downloads() {
    let app = Router::new().route("/clip.mp4", get(|| async { "fixture" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/clip.mp4", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    engine
        .update_settings(Settings {
            auto_start_small_jobs: false,
            ..Settings::default()
        })
        .await
        .unwrap();
    let mut source = config(url);
    source.kind = SourceKind::Media;
    let created = engine.create(source).await.unwrap();
    let mut checkpoint = wait(&engine, &created.id, "review").await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    server.abort();
    let directory = temp.path().join(&created.id);
    tokio::fs::create_dir_all(directory.join("media"))
        .await
        .unwrap();
    let payload = include_bytes!("fixtures/sync-bframes-video.mp4");
    tokio::fs::write(directory.join("media/saved.mp4"), payload)
        .await
        .unwrap();
    checkpoint.status = "failed".into();
    checkpoint.phase = "package".into();
    checkpoint.error = Some("Interrupted packaging".into());
    checkpoint.entries[0].state = "done".into();
    checkpoint.entries[0].media_file = Some("saved.mp4".into());
    checkpoint.entries[0].size = Some(payload.len() as u64);
    checkpoint.retry_at = Some(zim_engine::now() + 1);
    tokio::fs::write(
        directory.join("job.json"),
        serde_json::to_vec(&checkpoint).unwrap(),
    )
    .await
    .unwrap();
    let recovered = Engine::open(temp.path()).await.unwrap();
    let mut updated = checkpoint.config.clone();
    updated.title = "A revised title".into();
    updated.delay_ms = 3000;
    let retry = recovered.edit_retry(&created.id, updated).await.unwrap();
    assert_eq!(retry.phase, "package");
    assert_eq!(
        retry.retry_at, checkpoint.retry_at,
        "editing must not bypass saved server backoff"
    );
    let completed = wait(&recovered, &created.id, "completed").await;
    assert_eq!(completed.requests, checkpoint.requests);
    assert_eq!(completed.entries[0].id, checkpoint.entries[0].id);
    assert_eq!(completed.config.delay_ms, 3000);
    let archive = zimru::Archive::open(recovered.artifact(&created.id).await.unwrap()).unwrap();
    assert_eq!(archive.get_bytes("saved.mp4").unwrap(), payload);
    assert!(archive
        .get_text("index.html")
        .unwrap()
        .contains("A revised title"));
}

async fn saved_attempt(root: &std::path::Path, id: &str, status: &str) -> Job {
    let dir = root.join(id);
    tokio::fs::create_dir_all(dir.join("cache")).await.unwrap();
    let mut job: Job = serde_json::from_value(serde_json::json!({
        "id":id,"config":config("https://example.test/".into()),
        "status":status,"phase":"download","created_at":1,"updated_at":1,
        "entries":[{"id":"home","url":"https://example.test/","title":"Home","depth":0,"selected":true,"state":"pending","size":50,"duration":null,"error":null,"text":"Offline notes","assets":["https://cdn.example.test/style.css","https://cdn.example.test/font.woff2"],"media_file":null}],
        "queue":[],"visited":[],"resources":{},"logs":[],"error":"Asset unavailable","output":null,"output_bytes":null,"retry_at":null,"requests":9,"bytes":50
    })).unwrap();
    for (name, state) in [("style.css", "done"), ("font.woff2", "failed")] {
        let url = format!("https://cdn.example.test/{name}");
        job.resources.insert(
            url.clone(),
            zim_engine::Resource {
                url,
                path: name.into(),
                state: state.into(),
                error: None,
                mime: if name.ends_with("css") {
                    "text/css"
                } else {
                    "font/woff2"
                }
                .into(),
                dependencies: vec![],
                etag: None,
                last_modified: None,
                size: 1,
            },
        );
    }
    tokio::fs::write(dir.join("cache/home.html"), "<!doctype html><title>Home</title><link rel='stylesheet' href='https://cdn.example.test/style.css'><p>Offline notes</p>").await.unwrap();
    tokio::fs::write(dir.join("cache/style.css"), "body{color:navy}")
        .await
        .unwrap();
    tokio::fs::write(dir.join("job.json"), serde_json::to_vec(&job).unwrap())
        .await
        .unwrap();
    job
}

#[tokio::test]
async fn failed_attempt_deletion_removes_only_its_internal_files_and_survives_restart() {
    let temp = tempfile::tempdir().unwrap();
    saved_attempt(temp.path(), "failed", "failed").await;
    saved_attempt(temp.path(), "paused", "paused").await;
    let export = temp.path().join("exported.zim");
    tokio::fs::write(&export, "exported copy").await.unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    assert!(engine.delete_failed("paused").await.is_err());
    let failed = engine.record("failed").await.unwrap();
    failed.running.store(true, Ordering::SeqCst);
    assert!(engine.delete_failed("failed").await.is_err());
    failed.running.store(false, Ordering::SeqCst);
    assert!(engine.delete_failed("failed").await.unwrap());
    assert!(engine.get("failed").await.is_err());
    assert!(!temp.path().join("failed").exists());
    assert_eq!(failed.job.lock().await.status, "deleted");
    assert!(engine.resume("failed").await.is_err());
    assert_eq!(
        tokio::fs::read_to_string(export).await.unwrap(),
        "exported copy"
    );
    let reopened = Engine::open(temp.path()).await.unwrap();
    assert_eq!(reopened.list().await.len(), 1);
    assert!(reopened.get("paused").await.is_ok());
}

#[tokio::test]
async fn available_files_recovery_keeps_pages_and_saved_assets_with_visible_omission_report() {
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = saved_attempt(temp.path(), "partial", "paused").await;
    fixture.retry_at = Some(zim_engine::now() + 3600);
    tokio::fs::write(
        temp.path().join("partial/job.json"),
        serde_json::to_vec(&fixture).unwrap(),
    )
    .await
    .unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let summary = engine
        .get("partial")
        .await
        .unwrap()
        .view()
        .asset_summary
        .unwrap();
    assert_eq!((summary.downloaded, summary.missing), (1, 1));
    engine.build_available("partial").await.unwrap();
    let done = wait(&engine, "partial", "completed").await;
    assert_eq!(
        done.requests, fixture.requests,
        "recovery must not make network requests"
    );
    let archive = zimru::Archive::open(engine.artifact("partial").await.unwrap()).unwrap();
    assert!(archive
        .get_text("index.html")
        .unwrap()
        .contains("View capture report"));
    let report = archive.get_text("capture-report.html").unwrap();
    assert!(report.contains("1 assets were omitted"));
    assert!(report.contains("cdn.example.test/font.woff2"));
    assert!(archive
        .get_text("home.html")
        .unwrap()
        .contains("Offline notes"));
    assert!(archive
        .get_text("style.css")
        .unwrap()
        .contains("color:navy"));
    assert_eq!(
        done.resources["https://cdn.example.test/font.woff2"].state,
        "skipped"
    );
    assert!(temp.path().join("partial/recovery-history").is_dir());
    assert!(engine.build_available("partial").await.is_err());
}

#[tokio::test]
async fn available_files_recovery_rejects_missing_selected_page_before_changing_checkpoint() {
    let temp = tempfile::tempdir().unwrap();
    saved_attempt(temp.path(), "missing", "failed").await;
    tokio::fs::remove_file(temp.path().join("missing/cache/home.html"))
        .await
        .unwrap();
    tokio::fs::write(temp.path().join("missing/archive.zim"), "previous archive")
        .await
        .unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    assert!(engine
        .build_available("missing")
        .await
        .unwrap_err()
        .to_string()
        .contains("Saved page is missing"));
    let job = engine.get("missing").await.unwrap();
    assert_eq!(job.status, "failed");
    assert_eq!(job.phase, "download");
    assert_eq!(
        job.resources["https://cdn.example.test/font.woff2"].state,
        "failed"
    );
    assert_eq!(
        tokio::fs::read_to_string(temp.path().join("missing/archive.zim"))
            .await
            .unwrap(),
        "previous archive"
    );
}

#[tokio::test]
async fn refresh_reviews_changes_before_rebuilding_and_preserves_previous_archive() {
    let generation = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/robots.txt", get(|| async { "User-agent: *\nAllow: /" }))
        .route(
            "/",
            get(|State(g): State<Arc<AtomicUsize>>| async move {
                if g.load(Ordering::SeqCst) == 0 {
                    Html("<title>Collection</title><a href='/old'>Old page</a>")
                } else {
                    Html("<title>Updated collection</title><a href='/new'>New page</a>")
                }
            }),
        )
        .route(
            "/old",
            get(|| async { Html("<title>Old page</title><p>Earlier notes</p>") }),
        )
        .route(
            "/new",
            get(|| async { Html("<title>New page</title><p>New notes</p>") }),
        )
        .with_state(generation.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let created = engine.create(config(url.clone())).await.unwrap();
    wait(&engine, &created.id, "completed").await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let previous = tokio::fs::read(temp.path().join(&created.id).join("archive.zim"))
        .await
        .unwrap();
    generation.store(1, Ordering::SeqCst);
    engine.refresh_collection(&created.id).await.unwrap();
    let review = wait(&engine, &created.id, "review").await;
    assert_eq!(review.archive_revision, 1);
    let refresh = review.refresh.unwrap();
    assert!(refresh.complete);
    assert!(refresh
        .changes
        .iter()
        .any(|c| c.url == url && c.kind == "changed"));
    assert!(refresh
        .changes
        .iter()
        .any(|c| c.url.ends_with("/new") && c.kind == "new"));
    assert!(refresh
        .changes
        .iter()
        .any(|c| c.url.ends_with("/old") && c.kind == "not_seen"));
    assert_eq!(
        tokio::fs::read(temp.path().join(&created.id).join("archive.zim"))
            .await
            .unwrap(),
        previous
    );
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    engine
        .build(
            &created.id,
            review.entries.iter().map(|e| e.id.clone()).collect(),
        )
        .await
        .unwrap();
    wait(&engine, &created.id, "completed").await;
    let archive = zimru::Archive::open(engine.artifact(&created.id).await.unwrap()).unwrap();
    let index = archive.get_text("index.html").unwrap();
    assert!(index.contains("New page"));
    assert!(!index.contains("Old page"));
    server.abort();
}

#[tokio::test]
async fn refresh_reuses_saved_youtube_media_and_marks_incomplete_scan_without_claiming_removal() {
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = saved_attempt(temp.path(), "refresh-media", "paused").await;
    fixture.config.kind = SourceKind::Media;
    fixture.config.url = "https://www.youtube.com/playlist?list=fixture".into();
    let mut previous = fixture.entries[0].clone();
    previous.url = "https://www.youtube.com/watch?v=fixture1234".into();
    previous.state = "done".into();
    previous.media_file = Some("saved.mp4".into());
    previous.media_audio = Some("saved-audio.m4a".into());
    fixture.entries[0] = previous.clone();
    fixture.entries[0].state = "pending".into();
    fixture.entries[0].media_file = None;
    fixture.entries[0].media_audio = None;
    let mut unseen = previous.clone();
    unseen.url = "https://www.youtube.com/watch?v=fixture5678".into();
    fixture.refresh = Some(zim_engine::refresh::RefreshState {
        previous: vec![previous, unseen],
        ..Default::default()
    });
    fixture.scan_truncated = true;
    let dir = temp.path().join("refresh-media");
    tokio::fs::create_dir_all(dir.join("media")).await.unwrap();
    tokio::fs::write(dir.join("media/saved.mp4"), "video")
        .await
        .unwrap();
    tokio::fs::write(dir.join("media/saved-audio.m4a"), "audio")
        .await
        .unwrap();
    tokio::fs::write(dir.join("job.json"), serde_json::to_vec(&fixture).unwrap())
        .await
        .unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let record = engine.record("refresh-media").await.unwrap();
    zim_engine::refresh::compare(&record).await.unwrap();
    let job = engine.get("refresh-media").await.unwrap();
    assert_eq!(job.entries[0].state, "done");
    assert_eq!(job.entries[0].media_file.as_deref(), Some("saved.mp4"));
    assert_eq!(
        job.entries[0].media_audio.as_deref(),
        Some("saved-audio.m4a")
    );
    let refresh = job.refresh.unwrap();
    assert!(!refresh.complete);
    assert_eq!(
        refresh
            .changes
            .iter()
            .filter(|c| c.kind == "not_seen")
            .count(),
        1
    );
}

#[tokio::test]
async fn scope_asset_switches_and_budget_preserve_pages_before_large_assets() {
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/robots.txt",get(|| async { "User-agent: *\nAllow: /" }))
        .route("/",get(|| async { Html("<title>Home</title><a href='/news/story'>Story</a><a href='/news/private'>Private</a><a href='/about'>About</a>") }))
        .route("/news/story",get(|| async { Html("<title>Story</title><p>Important saved text.</p><img src='/large.png'><link rel='stylesheet' href='/font.woff2'>") }))
        .route("/news/private",get(|| async { panic!("excluded page was requested"); #[allow(unreachable_code)] "private" }))
        .route("/about",get(|| async { panic!("out-of-include page was requested"); #[allow(unreachable_code)] "about" }))
        .route("/font.woff2",get(|| async { panic!("disabled font was requested"); #[allow(unreachable_code)] "font" }))
        .route("/large.png",get(|State(hits):State<Arc<AtomicUsize>>| async move { hits.fetch_add(1,Ordering::SeqCst); ([("Content-Type","image/png")], vec![0u8; 2*1024*1024]) }))
        .with_state(hits.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let mut options = config(url);
    options.capture.include_urls = vec!["/news/*".into()];
    options.capture.exclude_urls = vec!["/news/private".into()];
    options.capture.fonts = false;
    options.capture.max_download_mb = Some(1);
    let created = engine.create(options).await.unwrap();
    let review = wait(&engine, &created.id, "review").await;
    assert_eq!(review.entries.len(), 2);
    assert!(
        !review.entries[0].selected,
        "seed page is discovery-only when not included"
    );
    let selected = review
        .entries
        .iter()
        .filter(|e| e.selected)
        .map(|e| e.id.clone())
        .collect();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    engine.build(&created.id, selected).await.unwrap();
    let failed = wait(&engine, &created.id, "failed").await;
    assert!(failed.downloaded_bytes < 1024 * 1024);
    assert_eq!(
        failed
            .resources
            .values()
            .filter(|r| r.state == "excluded")
            .count(),
        1
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    engine.build_available(&created.id).await.unwrap();
    wait(&engine, &created.id, "completed").await;
    let archive = zimru::Archive::open(engine.artifact(&created.id).await.unwrap()).unwrap();
    assert!(archive.get_text("index.html").unwrap().contains("Story"));
    assert!(archive
        .get_text("capture-report.html")
        .unwrap()
        .contains("large.png"));
    server.abort();
}

#[tokio::test]
async fn caption_archive_packages_vtt_full_transcript_and_search_timestamps_offline() {
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = saved_attempt(temp.path(), "caption-fixture", "completed").await;
    fixture.config.kind = SourceKind::Media;
    fixture.config.url = "https://media.example.test/movie.mp4".into();
    fixture.resources.clear();
    fixture.phase = "package".into();
    fixture.entries[0].state = "done".into();
    fixture.entries[0].assets.clear();
    fixture.entries[0].media_file = Some("saved.mp4".into());
    fixture.entries[0].transcript_file = Some("transcript.json".into());
    fixture.entries[0].subtitles = vec![zim_engine::captions::SavedSubtitle {
        file: "captions.vtt".into(),
        language: "en".into(),
        label: "English".into(),
        auto_generated: false,
    }];
    let cues = vec![zim_engine::captions::Cue {
        start: 1.25,
        duration: 2.0,
        text: "Moonlight botanists <script> & stars".into(),
    }];
    let dir = temp.path().join("caption-fixture");
    tokio::fs::create_dir_all(dir.join("media")).await.unwrap();
    tokio::fs::write(dir.join("media/saved.mp4"), "fixture video")
        .await
        .unwrap();
    tokio::fs::write(
        dir.join("media/transcript.json"),
        serde_json::to_vec(&cues).unwrap(),
    )
    .await
    .unwrap();
    tokio::fs::write(
        dir.join("media/captions.vtt"),
        zim_engine::captions::vtt(&cues),
    )
    .await
    .unwrap();
    tokio::fs::write(dir.join("job.json"), serde_json::to_vec(&fixture).unwrap())
        .await
        .unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    engine
        .rebuild("caption-fixture", ArchiveTemplate::ScreeningRoom)
        .await
        .unwrap();
    let done = wait(&engine, "caption-fixture", "completed").await;
    assert_eq!(done.requests, fixture.requests);
    let archive = zimru::Archive::open(engine.artifact("caption-fixture").await.unwrap()).unwrap();
    let index = archive.get_text("index.html").unwrap();
    assert!(index.contains("moonlight"));
    assert!(index.contains("\\\"start\\\":1.25") || index.contains("\"start\":1.25"));
    let page = archive.get_text("home.html").unwrap();
    assert!(page.contains("kind='subtitles' src='captions.vtt'"));
    assert!(page.contains("href='#t=1.25'"));
    assert!(page.contains("&lt;script&gt; &amp; stars"));
    assert!(!page.contains("botanists <script>"));
    assert!(archive
        .get_text("captions.vtt")
        .unwrap()
        .contains("00:00:01.250"));
    assert!(archive
        .get_text("media.js")
        .unwrap()
        .contains("currentTime"));
    if let Some(destination) = std::env::var_os("ZIMSTUDIO_CAPTIONS_OUTPUT_DIR") {
        tokio::fs::create_dir_all(&destination).await.unwrap();
        let mut files = tokio::fs::read_dir(dir.join("content")).await.unwrap();
        while let Some(file) = files.next_entry().await.unwrap() {
            tokio::fs::copy(
                file.path(),
                std::path::Path::new(&destination).join(file.file_name()),
            )
            .await
            .unwrap();
        }
    }
}

fn import_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::{Cursor, Write};
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        writer
            .start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
async fn upload(engine: &Engine, config: NewJob, name: &str, bytes: &[u8]) -> Job {
    let job = engine
        .begin_import(
            config,
            vec![zim_engine::imports::ImportFile {
                path: name.into(),
                size: bytes.len() as u64,
            }],
            None,
        )
        .await
        .unwrap();
    for (index, chunk) in bytes.chunks(256 * 1024).enumerate() {
        engine
            .import_chunk(&job.id, name, (index * 256 * 1024) as u64, chunk)
            .await
            .unwrap();
    }
    job
}

#[tokio::test]
async fn persists_compression_per_job_and_recompresses_saved_content_at_the_current_level() {
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    engine
        .update_settings(Settings {
            zstd_level: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    let html = format!(
        "<title>Compression fixture</title><p>{}</p>",
        (0..2000)
            .map(|i| format!("Garden {} has {} plants. ", i % 137, i % 31))
            .collect::<String>()
    );
    let job = upload(
        &engine,
        config("https://local-import.invalid/".into()),
        "index.html",
        html.as_bytes(),
    )
    .await;
    let review = engine.finish_import(&job.id).await.unwrap();
    assert_eq!(review.zstd_level, 1);
    engine
        .update_settings(Settings {
            zstd_level: 9,
            ..Default::default()
        })
        .await
        .unwrap();
    engine
        .build(
            &job.id,
            review.entries.iter().map(|e| e.id.clone()).collect(),
        )
        .await
        .unwrap();
    let original = wait(&engine, &job.id, "completed").await;
    assert_eq!(
        original.zstd_level, 1,
        "Changing preferences must not change an existing job"
    );
    let path = temp.path().join(&job.id).join("archive.zim");
    let index = zimru::Archive::open(&path)
        .unwrap()
        .get_text("index.html")
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let reopened = Engine::open(temp.path()).await.unwrap();
    assert_eq!(reopened.settings().await.zstd_level, 9);
    assert_eq!(reopened.get(&job.id).await.unwrap().zstd_level, 1);
    reopened
        .rebuild(&job.id, ArchiveTemplate::Atlas)
        .await
        .unwrap();
    let rebuilt = wait(&reopened, &job.id, "completed").await;
    assert_eq!(rebuilt.zstd_level, 9);
    assert_eq!(rebuilt.requests, 0, "Recompression must remain offline");
    assert_eq!(
        zimru::Archive::open(&path)
            .unwrap()
            .get_text("index.html")
            .unwrap(),
        index
    );
    assert!(
        rebuilt.output_bytes.unwrap() < original.output_bytes.unwrap(),
        "Higher compression must produce a smaller fixture ZIM with identical content"
    );
    assert!(rebuilt.validation.unwrap().checksum_verified);
}

#[tokio::test]
async fn imports_saved_zip_with_assets_offline_and_can_rebuild_without_uploaded_staging() {
    let html=b"<title>Imported garden</title><link rel='stylesheet' href='assets/style.css'><p>Locally saved botany.</p><img src='missing.png'>";
    let zip = import_zip(&[
        ("saved/index.html", html),
        ("saved/assets/style.css", b"body{color:green}"),
    ]);
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let job = upload(
        &engine,
        config("https://example.test/".into()),
        "saved.zip",
        &zip,
    )
    .await;
    let review = engine.finish_import(&job.id).await.unwrap();
    assert_eq!(review.status, "review");
    assert_eq!(review.requests, 0);
    assert!(review.import.as_ref().unwrap().finished);
    assert!(!temp.path().join(&job.id).join("uploads").exists());
    assert_eq!(review.entries.len(), 1);
    assert_eq!(review.resources.len(), 1);
    engine
        .build(
            &job.id,
            review.entries.iter().map(|e| e.id.clone()).collect(),
        )
        .await
        .unwrap();
    let done = wait(&engine, &job.id, "completed").await;
    assert_eq!(
        done.requests, 0,
        "local imports must never fetch missing resources"
    );
    let archive = zimru::Archive::open(engine.artifact(&job.id).await.unwrap()).unwrap();
    assert!(archive
        .get_text("index.html")
        .unwrap()
        .contains("Imported garden"));
    assert!(archive
        .get_text("index.html")
        .unwrap()
        .contains("Imported saved files"));
    assert!(archive
        .get_text("capture-report.html")
        .unwrap()
        .contains("missing.png"));
    let page = archive
        .get_text(&format!("{}.html", review.entries[0].id))
        .unwrap();
    assert!(page.contains("Locally saved botany"));
    assert!(!page.contains("href='assets/style.css'"));
    let css = review.resources.values().next().unwrap();
    assert!(archive.get_text(&css.path).unwrap().contains("color:green"));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    engine
        .rebuild(&job.id, ArchiveTemplate::FieldNotes)
        .await
        .unwrap();
    assert_eq!(wait(&engine, &job.id, "completed").await.requests, 0);
}
#[tokio::test]
async fn interrupted_import_resumes_exact_saved_bytes_and_rejects_unapproved_paths() {
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let html = b"<title>Resumed page</title><p>Saved content</p>";
    let files = vec![zim_engine::imports::ImportFile {
        path: "index.html".into(),
        size: html.len() as u64,
    }];
    let settings = config("https://example.test/".into());
    let job = engine
        .begin_import(settings.clone(), files.clone(), None)
        .await
        .unwrap();
    engine
        .import_chunk(&job.id, "index.html", 0, &html[..10])
        .await
        .unwrap();
    assert!(engine
        .import_chunk(&job.id, "../escape", 0, b"x")
        .await
        .is_err());
    assert!(engine
        .import_chunk(&job.id, "index.html", 0, b"x")
        .await
        .is_err());
    drop(engine);
    let engine = Engine::open(temp.path()).await.unwrap();
    assert_eq!(engine.get(&job.id).await.unwrap().status, "paused");
    assert!(engine.resume(&job.id).await.is_err());
    engine
        .begin_import(settings, files, Some(job.id.clone()))
        .await
        .unwrap();
    assert_eq!(
        engine.import_offset(&job.id, "index.html").await.unwrap(),
        10
    );
    engine
        .import_chunk(&job.id, "index.html", 10, &html[10..])
        .await
        .unwrap();
    assert_eq!(
        engine.finish_import(&job.id).await.unwrap().entries[0].title,
        "Resumed page"
    );
}
#[tokio::test]
async fn zip_import_rejects_traversal_case_aliases_and_expansion_beyond_budget() {
    for files in [
        vec![("../outside.html", b"<title>Escape</title>".as_slice())],
        vec![
            ("index.html", b"<title>A</title>".as_slice()),
            ("INDEX.html", b"<title>B</title>".as_slice()),
        ],
    ] {
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::open(temp.path()).await.unwrap();
        let zip = import_zip(&files);
        let job = upload(
            &engine,
            config("https://example.test/".into()),
            "unsafe.zip",
            &zip,
        )
        .await;
        assert!(engine.finish_import(&job.id).await.is_err());
        assert_eq!(engine.get(&job.id).await.unwrap().status, "failed");
        assert!(!temp.path().join("outside.html").exists());
        assert!(engine.delete_failed(&job.id).await.unwrap());
    }
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let bytes = vec![b'x'; 2 * 1024 * 1024];
    let zip = import_zip(&[("large.html", &bytes)]);
    let mut options = config("https://example.test/".into());
    options.capture.max_download_mb = Some(1);
    let job = upload(&engine, options, "large.zip", &zip).await;
    assert!(engine
        .finish_import(&job.id)
        .await
        .unwrap_err()
        .to_string()
        .contains("budget"));
}
#[tokio::test]
async fn url_list_crawls_each_seed_scope_and_does_not_fetch_outside_paths() {
    let app = Router::new()
        .route("/robots.txt", get(|| async { "User-agent: *\nAllow: /" }))
        .route(
            "/first/",
            get(|| async { Html("<title>First</title><a href='/outside'>Outside</a>") }),
        )
        .route("/second/", get(|| async { Html("<title>Second</title>") }))
        .route(
            "/outside",
            get(|| async {
                panic!("out-of-scope URL was fetched");
                #[allow(unreachable_code)]
                "outside"
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let mut options = config(format!("{base}/first/"));
    options.capture.seed_urls = vec![format!("{base}/second/")];
    let created = engine.create(options).await.unwrap();
    let done = wait(&engine, &created.id, "completed").await;
    assert_eq!(done.entries.len(), 2);
    assert!(done.entries.iter().any(|e| e.title == "Second"));
    server.abort();
}

#[tokio::test]
async fn website_refresh_revalidates_same_url_assets_and_reuses_unchanged_responses() {
    use axum::http::HeaderMap;
    let generation = Arc::new(AtomicUsize::new(0));
    let conditional = Arc::new(AtomicUsize::new(0));
    let state = (generation.clone(), conditional.clone());
    let app=Router::new().route("/robots.txt",get(||async{"User-agent: *\nAllow: /"}))
      .route("/",get(||async{Html("<title>Asset refresh</title><p>Stable page text.</p><link rel='stylesheet' href='/style.css'><img src='/image.svg'>")}))
      .route("/style.css",get(|headers:HeaderMap,State((_generation,count)):State<(Arc<AtomicUsize>,Arc<AtomicUsize>)>|async move{
        if headers.get("if-none-match").and_then(|v|v.to_str().ok())==Some("\"style-stable\""){count.fetch_add(1,Ordering::SeqCst);return StatusCode::NOT_MODIFIED.into_response();}
        ([("Content-Type","text/css"),("ETag","\"style-stable\"")],"body{color:blue}").into_response()
      }))
      .route("/image.svg",get(|headers:HeaderMap,State((generation,count)):State<(Arc<AtomicUsize>,Arc<AtomicUsize>)>|async move{
        if headers.contains_key("if-none-match"){count.fetch_add(1,Ordering::SeqCst);}
        let new=generation.load(Ordering::SeqCst)>0;
        ([("Content-Type","image/svg+xml"),("ETag",if new{"\"image-new\""}else{"\"image-old\""})],if new{"<svg>updated</svg>"}else{"<svg>original</svg>"}).into_response()
      })).with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let created = engine.create(config(base.clone())).await.unwrap();
    wait(&engine, &created.id, "completed").await;
    generation.store(1, Ordering::SeqCst);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    engine.refresh_collection(&created.id).await.unwrap();
    let review = wait(&engine, &created.id, "review").await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    engine
        .build(
            &created.id,
            review
                .entries
                .iter()
                .filter(|e| e.selected)
                .map(|e| e.id.clone())
                .collect(),
        )
        .await
        .unwrap();
    let done = wait(&engine, &created.id, "completed").await;
    assert_eq!(
        conditional.load(Ordering::SeqCst),
        2,
        "both saved assets should be revalidated"
    );
    let archive = zimru::Archive::open(engine.artifact(&created.id).await.unwrap()).unwrap();
    let image = done.resources.get(&format!("{base}image.svg")).unwrap();
    let css = done.resources.get(&format!("{base}style.css")).unwrap();
    assert!(archive.get_text(&image.path).unwrap().contains("updated"));
    assert!(archive.get_text(&css.path).unwrap().contains("color:blue"));
    assert!(done.validation.unwrap().checksum_verified);
    server.abort();
}

#[tokio::test]
async fn validation_detects_checksum_and_directory_corruption_without_touching_the_good_archive() {
    use md5::{Digest, Md5};
    let temp = tempfile::tempdir().unwrap();
    saved_attempt(temp.path(), "validate", "paused").await;
    let engine = Engine::open(temp.path()).await.unwrap();
    engine.build_available("validate").await.unwrap();
    wait(&engine, "validate", "completed").await;
    let record = engine.record("validate").await.unwrap();
    let good = tokio::fs::read(engine.artifact("validate").await.unwrap())
        .await
        .unwrap();
    let corrupt = temp.path().join("corrupt.zim");
    let mut bytes = good.clone();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    tokio::fs::write(&corrupt, &bytes).await.unwrap();
    assert!(zim_engine::capture_report::validate(&record, &corrupt)
        .await
        .unwrap_err()
        .to_string()
        .contains("checksum"));
    bytes = good.clone();
    bytes[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
    let end = bytes.len() - 16;
    let digest = Md5::digest(&bytes[..end]);
    bytes[end..].copy_from_slice(&digest);
    tokio::fs::write(&corrupt, &bytes).await.unwrap();
    assert!(zim_engine::capture_report::validate(&record, &corrupt)
        .await
        .is_err());
    assert_eq!(
        tokio::fs::read(engine.artifact("validate").await.unwrap())
            .await
            .unwrap(),
        good
    );
}
