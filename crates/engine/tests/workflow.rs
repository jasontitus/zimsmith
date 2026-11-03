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
    }
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
    let fixture = saved_attempt(temp.path(), "partial", "paused").await;
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
