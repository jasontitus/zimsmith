//! Optional, bounded artwork. Missing artwork must never discard saved media.
use crate::{
    web::{http_url, Network},
    ArchiveTemplate, Record, SourceKind,
};
use anyhow::Result;

fn image_extension(body: &[u8]) -> Option<&'static str> {
    if body.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if body.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if body.starts_with(b"RIFF") && body.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else {
        None
    }
}

pub async fn prepare(record: &Record) -> Result<()> {
    let job = record.job.lock().await.clone();
    if job.archive_template != ArchiveTemplate::ScreeningRoom
        || job.config.kind != SourceKind::Media
    {
        return Ok(());
    }
    let mut network = Network::new(record).await?.with_cap(2 * 1024 * 1024);
    let directory = record.dir.join("media");
    tokio::fs::create_dir_all(&directory).await?;
    let entries: Vec<_> = job
        .entries
        .iter()
        .filter(|e| e.selected && e.state == "done")
        .collect();
    for (i, entry) in entries.iter().enumerate() {
        if let Some(file) = &entry.thumbnail_file {
            if directory.join(file).is_file() {
                continue;
            }
        }
        // Older manifests predate thumbnail metadata; recover standard YouTube artwork.
        let thumbnail_url = entry
            .thumbnail_url
            .clone()
            .or_else(|| crate::native_media::youtube_thumbnail(&entry.url));
        let Some(url) = &thumbnail_url else {
            continue;
        };
        record
            .progress(
                format!("Saving gallery artwork: {}", entry.title),
                i as u64,
                Some(entries.len() as u64),
                "thumbnails",
            )
            .await;
        let result: Result<String> = async {
            let (_, _, body) = network.get_asset(&http_url(url)?).await?;
            let ext = image_extension(&body)
                .ok_or_else(|| anyhow::anyhow!("Thumbnail is not a supported image"))?;
            let filename = format!("{}-poster.{ext}", entry.id);
            let partial = directory.join(format!("{filename}.partial"));
            tokio::fs::write(&partial, body).await?;
            tokio::fs::rename(partial, directory.join(&filename)).await?;
            Ok(filename)
        }
        .await;
        match result {
            Ok(filename) => {
                record
                    .change(|j| {
                        if let Some(item) = j.entries.iter_mut().find(|e| e.id == entry.id) {
                            item.thumbnail_file = Some(filename);
                            item.thumbnail_url = thumbnail_url;
                        }
                    })
                    .await?
            }
            Err(error) => {
                record
                    .log(
                        "warning",
                        format!(
                            "Artwork unavailable for {}: {error}. Using the gallery placeholder.",
                            entry.title
                        ),
                    )
                    .await?
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artwork_accepts_raster_images_and_rejects_html_or_svg() {
        assert_eq!(image_extension(b"\xff\xd8\xffdata"), Some("jpg"));
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\n"), Some("png"));
        assert_eq!(image_extension(b"RIFFxxxxWEBP"), Some("webp"));
        assert_eq!(image_extension(b"<html>Forbidden</html>"), None);
        assert_eq!(image_extension(b"<svg onload='evil()'>"), None);
        assert_eq!(
            crate::native_media::youtube_thumbnail("https://www.youtube.com/watch?v=O6VZaBM2Zr4"),
            Some("https://i.ytimg.com/vi/O6VZaBM2Zr4/hqdefault.jpg".into())
        );
        assert!(crate::native_media::youtube_thumbnail(
            "https://youtube.com.evil.test/watch?v=O6VZaBM2Zr4"
        )
        .is_none());
    }
    #[tokio::test]
    async fn gallery_embeds_artwork_once_and_survives_missing_images() {
        use crate::{Engine, NewJob, Settings};
        use axum::{routing::get, Router};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let hits = Arc::new(AtomicUsize::new(0));
        let count = hits.clone();
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 2, 2);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[120; 12])
                .unwrap();
        }
        let image = png.clone();
        let app = Router::new()
            .route("/robots.txt", get(|| async { "User-agent: *\nAllow: /" }))
            .route("/clip.mp4", get(|| async { "fixture media" }))
            .route(
                "/poster.png",
                get(move || {
                    let image = image.clone();
                    let count = count.clone();
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        ([("Content-Type", "image/png")], image)
                    }
                }),
            )
            .route(
                "/missing.jpg",
                get(|| async { axum::http::StatusCode::FORBIDDEN }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let temp = tempfile::tempdir().unwrap();
        let engine = Engine::open(temp.path()).await.unwrap();
        engine
            .update_settings(Settings {
                auto_start_small_jobs: false,
                archive_template: ArchiveTemplate::ScreeningRoom,
                ..Settings::default()
            })
            .await
            .unwrap();
        let job = engine
            .create(NewJob {
                url: format!("{origin}/clip.mp4"),
                title: "Video gallery".into(),
                kind: SourceKind::Media,
                max_items: 10,
                max_depth: 0,
                delay_ms: 1000,
                max_resource_mb: 2,
                language: "eng".into(),
                description: "Offline video collection".into(),
                creator: "Test".into(),
                include_assets: false,
                capture: Default::default(),
            })
            .await
            .unwrap();
        let record = engine.record(&job.id).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while record.running.load(Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        record
            .change(|j| {
                assert_eq!(j.entries.len(), 1);
                let e = &mut j.entries[0];
                e.state = "done".into();
                e.media_file = Some("clip.mp4".into());
                e.media_audio = Some("audio.m4a".into());
                e.duration = Some(752.);
                e.thumbnail_url = Some(format!("{origin}/poster.png"));
                let mut missing = e.clone();
                missing.id = "missing".into();
                missing.title = "No artwork".into();
                missing.thumbnail_url = Some(format!("{origin}/missing.jpg"));
                j.entries.push(missing);
            })
            .await
            .unwrap();
        tokio::fs::create_dir_all(record.dir.join("media"))
            .await
            .unwrap();
        tokio::fs::write(
            record.dir.join("media/clip.mp4"),
            include_bytes!("../tests/fixtures/sync-bframes-video.mp4"),
        )
        .await
        .unwrap();
        tokio::fs::write(
            record.dir.join("media/audio.m4a"),
            include_bytes!("../tests/fixtures/sync-audio.m4a"),
        )
        .await
        .unwrap();
        prepare(&record).await.unwrap();
        crate::archive::package(&record).await.unwrap();
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "saved artwork should be reused when packaging resumes"
        );
        let job = record.job.lock().await.clone();
        let archive = zimru::Archive::open(record.dir.join("archive.zim")).unwrap();
        let index = archive.get_text("index.html").unwrap();
        assert!(index.contains("screening_room") && index.contains("12:32"));
        assert!(index.contains("loading='lazy'") && index.contains("No artwork"));
        assert!(index.contains(&format!(
            "data-play-src='{}-combined.mp4'",
            job.entries[0].id
        )));
        assert!(archive
            .get_text("gallery.js")
            .unwrap()
            .contains("showModal"));
        let poster = job.entries[0].thumbnail_file.as_ref().unwrap();
        assert_eq!(
            archive
                .entry_by_ns_path(b'C', poster)
                .unwrap()
                .get_item(false)
                .unwrap()
                .bytes()
                .unwrap()
                .to_vec(),
            png
        );
        assert!(archive
            .get_text(&format!("{}.html", job.entries[0].id))
            .unwrap()
            .contains(&format!("poster=\"{poster}\"")));
        assert!(
            !index.contains(&format!("{origin}/poster.png"))
                && !index.contains(&format!("{origin}/missing.jpg")),
            "remote artwork URLs must not enter the search catalog"
        );
        assert!(job
            .logs
            .iter()
            .any(|l| l.message.contains("gallery placeholder")));
        server.abort();
        record.change(|j| j.phase = "package".into()).await.unwrap();
        engine
            .rebuild(&job.id, ArchiveTemplate::Midnight)
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let rebuilt = engine.get(&job.id).await.unwrap();
                assert_ne!(rebuilt.status, "failed", "{:?}", rebuilt.error);
                if rebuilt.status == "completed" {
                    assert_eq!(rebuilt.requests, job.requests);
                    assert_eq!(rebuilt.archive_revision, 1);
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let rebuilt = zimru::Archive::open(record.dir.join("archive.zim")).unwrap();
        let name = format!("{}-combined.mp4", job.entries[0].id);
        assert!(rebuilt.get_bytes(&name).unwrap().len() > 1000);
        assert!(rebuilt.get_bytes("audio.m4a").is_err());
        let page = rebuilt
            .get_text(&format!("{}.html", job.entries[0].id))
            .unwrap();
        assert!(page.contains(&name) && !page.contains("audio-track"));
        assert_eq!(
            tokio::fs::read(record.dir.join("media/clip.mp4"))
                .await
                .unwrap(),
            include_bytes!("../tests/fixtures/sync-bframes-video.mp4")
        );
        assert_eq!(
            tokio::fs::read(record.dir.join("media/audio.m4a"))
                .await
                .unwrap(),
            include_bytes!("../tests/fixtures/sync-audio.m4a")
        );
        assert!(rebuilt
            .get_text("index.html")
            .unwrap()
            .contains("data-zim-template=\"midnight\""));
    }
}
