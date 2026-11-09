use std::io::Write;
use zim_engine::imports::ImportFile;
use zim_engine::{Engine, NewJob};

fn config() -> NewJob {
    serde_json::from_value(serde_json::json!({"url":"https://local-import.invalid/", "title":"WARC fixture", "kind":"website", "max_items":10, "max_depth":1, "delay_ms":1000, "max_resource_mb":2, "language":"eng", "description":"fixture", "creator":"fixture", "include_assets":true})).unwrap()
}
fn record(kind: &str, url: &str, mime: &str, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!("WARC/1.1\r\nWARC-Type: {kind}\r\nWARC-Record-ID: <urn:uuid:00000000-0000-0000-0000-000000000001>\r\nWARC-Date: 2026-01-01T00:00:00Z\r\nWARC-Target-URI: {url}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n\r\n",body.len()).into_bytes();
    bytes.extend(body);
    bytes.extend(b"\r\n\r\n");
    bytes
}
fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}
async fn upload(engine: &Engine, name: &str, bytes: &[u8]) -> String {
    let job = engine
        .begin_import(
            config(),
            vec![ImportFile {
                path: name.into(),
                size: bytes.len() as u64,
            }],
            None,
        )
        .await
        .unwrap();
    for (i, chunk) in bytes.chunks(256 * 1024).enumerate() {
        engine
            .import_chunk(&job.id, name, (i * 256 * 1024) as u64, chunk)
            .await
            .unwrap();
    }
    job.id
}

#[tokio::test]
async fn warc_and_multimember_gzip_import_original_urls_assets_and_chunked_http_offline() {
    for compressed in [false, true] {
        let html=b"<title>Captured home</title><a href='/other'>Other</a><link rel='stylesheet' href='/style'><img src='/image'>";
        let zipped = gzip(html);
        let mut http=b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Encoding: X-Gzip\r\nTransfer-Encoding: Chunked\r\n\r\n".to_vec();
        http.extend(format!("{:x}\r\n", zipped.len()).as_bytes());
        http.extend(zipped);
        http.extend(b"\r\n0\r\n\r\n");
        let records = vec![
            record(
                "response",
                "https://capture.invalid/",
                "application/http; msgtype=response",
                &http,
            ),
            record(
                "resource",
                "https://capture.invalid/other",
                "text/html",
                b"<title>Other</title><p>Captured content</p>",
            ),
            record(
                "resource",
                "https://capture.invalid/style",
                "text/css",
                b"body { background:url('/image'); }",
            ),
            record(
                "resource",
                "https://capture.invalid/image",
                "image/svg+xml",
                b"<svg xmlns='http://www.w3.org/2000/svg'/>",
            ),
        ];
        let bytes: Vec<u8> = records
            .iter()
            .flat_map(|r| if compressed { gzip(r) } else { r.clone() })
            .collect();
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::open(temp.path()).await.unwrap();
        let id = upload(
            &engine,
            if compressed {
                "capture.warc.gz"
            } else {
                "capture.warc"
            },
            &bytes,
        )
        .await;
        let job = engine.finish_import(&id).await.unwrap();
        assert_eq!(job.status, "review");
        assert_eq!(job.entries.len(), 2);
        assert_eq!(job.resources.len(), 2);
        assert_eq!(job.requests, 0);
        assert_eq!(
            job.resources["https://capture.invalid/style"].mime,
            "text/css"
        );
        let home = job
            .entries
            .iter()
            .find(|e| e.url == "https://capture.invalid/")
            .unwrap()
            .id
            .clone();
        let other = job
            .entries
            .iter()
            .find(|e| e.url.ends_with("/other"))
            .unwrap()
            .id
            .clone();
        engine
            .build(&id, job.entries.iter().map(|e| e.id.clone()).collect())
            .await
            .unwrap();
        let done = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                let j = engine.get(&id).await.unwrap();
                if ["completed", "failed"].contains(&j.status.as_str()) {
                    break j;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(done.status, "completed", "{:?}", done.error);
        assert_eq!(done.requests, 0);
        let archive = zimru::Archive::open(done.output.unwrap()).unwrap();
        let saved = archive.get_text(&format!("{home}.html")).unwrap();
        assert!(saved.contains(&format!("{other}.html")));
        assert!(saved.contains(&job.resources["https://capture.invalid/image"].path));
    }
}

#[tokio::test]
async fn warc_rejects_truncation_oversize_headers_and_decoded_bombs_without_finishing_import() {
    let mut truncated = record(
        "resource",
        "https://capture.invalid/",
        "text/html",
        b"<title>Home</title>",
    );
    truncated.truncate(truncated.len() - 8);
    let oversized = format!("WARC/1.1\r\nX-Header: {}", "x".repeat(70 * 1024)).into_bytes();
    let body = gzip(&vec![b'x'; 3 * 1024 * 1024]);
    let mut http =
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Encoding: gzip\r\n\r\n".to_vec();
    http.extend(body);
    for bytes in [
        truncated,
        oversized,
        record("response", "https://capture.invalid/", "application/http", b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 999\r\n\r\n<title>Short</title>"),
        record(
            "response",
            "https://capture.invalid/",
            "application/http",
            &http,
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::open(temp.path()).await.unwrap();
        let id = upload(&engine, "bad.warc", &bytes).await;
        assert!(engine.finish_import(&id).await.is_err());
        let job = engine.get(&id).await.unwrap();
        assert_eq!(job.status, "failed");
        assert!(!job.import.unwrap().finished);
        assert_eq!(job.requests, 0);
    }
}

#[tokio::test]
async fn warc_last_complete_capture_wins_and_omissions_are_reported() {
    let mut bytes = record(
        "resource",
        "https://capture.invalid/",
        "text/html",
        b"<title>Old</title>",
    );
    bytes.extend(record(
        "resource",
        "https://capture.invalid/",
        "text/html",
        b"<title>Latest</title>",
    ));
    bytes.extend(record(
        "revisit",
        "https://capture.invalid/missing",
        "application/http",
        b"",
    ));
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let id = upload(&engine, "capture.warc", &bytes).await;
    let job = engine.finish_import(&id).await.unwrap();
    assert_eq!(job.entries.len(), 1);
    assert_eq!(job.entries[0].title, "Latest");
    assert!(job
        .logs
        .iter()
        .any(|l| l.message.starts_with("WARC import omitted 1")));
}

#[tokio::test]
async fn warc_budget_failure_can_retry_with_higher_limit_and_reuse_upload() {
    let mut body = b"<title>Large capture</title>".to_vec();
    body.extend(vec![b' '; 3 * 1024 * 1024]);
    let bytes = gzip(&record(
        "resource",
        "https://capture.invalid/",
        "text/html",
        &body,
    ));
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let id = upload(&engine, "capture.warc.gz", &bytes).await;
    assert!(engine.finish_import(&id).await.is_err());
    let mut larger = config();
    larger.max_resource_mb = 4;
    engine
        .begin_import(
            larger,
            vec![ImportFile {
                path: "capture.warc.gz".into(),
                size: bytes.len() as u64,
            }],
            Some(id.clone()),
        )
        .await
        .unwrap();
    assert_eq!(
        engine.import_offset(&id, "capture.warc.gz").await.unwrap(),
        bytes.len() as u64
    );
    let job = engine.finish_import(&id).await.unwrap();
    assert_eq!(job.status, "review");
    assert_eq!(job.entries[0].title, "Large capture");
    assert_eq!(job.requests, 0);
}

#[tokio::test]
async fn warc_container_can_exceed_individual_resource_limit() {
    let mut bytes = record(
        "resource",
        "https://capture.invalid/",
        "text/html",
        b"<title>Home</title>",
    );
    for path in ["one", "two"] {
        bytes.extend(record(
            "resource",
            &format!("https://capture.invalid/{path}"),
            "application/octet-stream",
            &vec![0; 1100 * 1024],
        ));
    }
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let id = upload(&engine, "capture.warc", &bytes).await;
    assert_eq!(engine.finish_import(&id).await.unwrap().resources.len(), 2);
}

#[tokio::test]
async fn warc_rejects_malformed_chunks_and_accepts_deflate_with_trailers() {
    for body in [
        "xyz\r\n<title>bad</title>",
        "5\r\nabc",
        "3\r\nabc\r\n",
        "3\r\nabc\r\n0\r\n\r\nextra",
    ] {
        let http = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\n\r\n{body}");
        let bytes = record(
            "response",
            "https://capture.invalid/",
            "application/http",
            http.as_bytes(),
        );
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::open(temp.path()).await.unwrap();
        let id = upload(&engine, "bad.warc", &bytes).await;
        assert!(
            engine.finish_import(&id).await.is_err(),
            "Accepted malformed chunk: {body}"
        );
        assert!(!engine.get(&id).await.unwrap().import.unwrap().finished);
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(b"<title>Deflate</title>").unwrap();
    let zipped = encoder.finish().unwrap();
    let mut http = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Encoding: deflate\r\nTransfer-Encoding: chunked\r\n\r\n{:x};ext=value\r\n", zipped.len()).into_bytes();
    http.extend(zipped);
    http.extend(b"\r\n0\r\nX-Fixture: complete\r\n\r\n");
    let bytes = record(
        "response",
        "https://capture.invalid/",
        "application/http",
        &http,
    );
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let id = upload(&engine, "capture.warc", &bytes).await;
    assert_eq!(
        engine.finish_import(&id).await.unwrap().entries[0].title,
        "Deflate"
    );
}

#[tokio::test]
async fn multiple_warcs_share_expansion_budget_even_for_ignored_metadata() {
    let bytes = gzip(&record(
        "warcinfo",
        "https://capture.invalid/",
        "application/warc-fields",
        &vec![b'x'; 1100 * 1024],
    ));
    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let mut settings = config();
    settings.capture.max_download_mb = Some(2);
    let files = ["one.warc.gz", "two.warc.gz"].map(|path| ImportFile {
        path: path.into(),
        size: bytes.len() as u64,
    });
    let job = engine
        .begin_import(settings, files.to_vec(), None)
        .await
        .unwrap();
    for file in files {
        engine
            .import_chunk(&job.id, &file.path, 0, &bytes)
            .await
            .unwrap();
    }
    let error = engine.finish_import(&job.id).await.unwrap_err();
    assert!(format!("{error:#}").contains("budget"));
    assert!(!engine.get(&job.id).await.unwrap().import.unwrap().finished);
}

#[tokio::test]
async fn warc_skips_bad_http_headers_and_keeps_assets_after_the_review_limit() {
    let partial = record(
        "resource",
        "https://capture.invalid/partial",
        "text/html",
        b"<title>Partial</title>",
    );
    let partial = partial.strip_prefix(b"WARC/1.1\r\n").unwrap();
    let mut bytes = b"WARC/1.1\r\nWARC-Truncated: length\r\n".to_vec();
    bytes.extend(partial);
    bytes.extend(record(
        "response",
        "https://capture.invalid/broken",
        "application/http",
        b"HTTP/1.1 200 OK\r\nBad Header: value\r\nContent-Type: text/html\r\n\r\n<title>Broken</title>",
    ));
    bytes.extend(record(
        "response",
        "https://capture.invalid/keep",
        "application/http",
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Security-Policy: default-src 'none';\r\n\timg-src 'self'\r\nX-Captured-Header: \xff\r\n\r\n<title>Keep</title><img src='/image'>",
    ));
    bytes.extend(record(
        "resource",
        "https://capture.invalid/extra",
        "text/html",
        b"<title>Beyond the limit</title>",
    ));
    bytes.extend(record(
        "resource",
        "https://capture.invalid/image",
        "image/svg+xml",
        b"<svg xmlns='http://www.w3.org/2000/svg'/>",
    ));

    let temp = tempfile::tempdir().unwrap();
    let engine = Engine::open(temp.path()).await.unwrap();
    let mut settings = config();
    settings.max_items = 1;
    let name = "sample.warc";
    let job = engine
        .begin_import(
            settings,
            vec![ImportFile {
                path: name.into(),
                size: bytes.len() as u64,
            }],
            None,
        )
        .await
        .unwrap();
    engine.import_chunk(&job.id, name, 0, &bytes).await.unwrap();
    let review = engine.finish_import(&job.id).await.unwrap();
    assert_eq!(review.status, "review");
    assert_eq!(review.entries.len(), 1);
    assert_eq!(review.entries[0].title, "Keep");
    assert!(review
        .resources
        .contains_key("https://capture.invalid/image"));
    assert_eq!(review.requests, 0);
    for warning in [
        "invalid captured HTTP headers",
        "beyond the 1-item review limit",
        "redirect, error, partial",
    ] {
        assert!(review.logs.iter().any(|log| log.message.contains(warning)));
    }
}
