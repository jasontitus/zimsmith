//! Rewrap encoded samples into one MP4 so a ZIM reader uses one playback clock.
//! Payloads are copied one sample at a time; no decoder, encoder, or process.
use crate::Record;
use anyhow::{Context, Result};
use mp4_track::{
    ChunkOffsetChoice, Edit, Limits, Mp4Reader, Mp4Writer, SampleInput, Track, TrackConfig,
    TrackKind, WriterConfig,
};
use std::{
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

pub async fn combine(
    record: &Record,
    video: PathBuf,
    audio: PathBuf,
    output: PathBuf,
) -> Result<()> {
    let total = tokio::fs::metadata(&video).await?.len() + tokio::fs::metadata(&audio).await?.len();
    let stop = Arc::new(AtomicBool::new(false));
    let _cancel = CancelOnDrop(stop.clone());
    let bytes = Arc::new(AtomicU64::new(0));
    let progress = bytes.clone();
    let mut task =
        tokio::task::spawn_blocking(move || remux(&video, &audio, &output, &stop, &progress));
    loop {
        tokio::select! {
            result = &mut task => {
                result.context("MP4 combining task failed")??;
                record.transfer_progress(bytes.load(Ordering::Relaxed), Some(total)).await;
                return Ok(());
            }
            _ = tokio::time::sleep(Duration::from_millis(250)) => record.transfer_progress(bytes.load(Ordering::Relaxed), Some(total)).await,
        }
    }
}

fn open(path: &Path) -> Result<Mp4Reader<BufReader<File>>> {
    let mut limits = Limits::default();
    limits.max_moov_bytes = 16 * 1024 * 1024;
    limits.max_moof_bytes = 2 * 1024 * 1024;
    limits.max_sample_bytes = 8 * 1024 * 1024;
    limits.max_fragments = 100_000;
    Mp4Reader::open_with(BufReader::new(File::open(path)?), limits).with_context(|| {
        format!(
            "Could not read saved MP4 track {}",
            path.file_name().unwrap_or_default().to_string_lossy()
        )
    })
}
fn scale(value: u64, from: u32, to: u32) -> Result<u64> {
    anyhow::ensure!(from > 0 && to > 0, "Invalid MP4 timescale");
    u64::try_from((u128::from(value) * u128::from(to) + u128::from(from / 2)) / u128::from(from))
        .context("MP4 timing overflow")
}
fn config(track: &Track, source_clock: u32, output_clock: u32, start: u64) -> Result<TrackConfig> {
    let mut edits = track.edit_list.clone();
    for edit in &mut edits {
        edit.segment_duration = scale(edit.segment_duration, source_clock, output_clock)?;
        if let Some(time) = &mut edit.media_time {
            *time = time
                .checked_sub(start)
                .context("Unsupported MP4 edit before its first sample")?;
        }
    }
    // Writer samples start at DTS zero. Preserve a delayed track's presentation.
    if edits.is_empty() && start > 0 {
        edits = vec![
            Edit {
                segment_duration: scale(start, track.timescale, output_clock)?,
                media_time: None,
                media_rate: 1.0,
            },
            Edit {
                segment_duration: scale(track.duration, track.timescale, output_clock)?,
                media_time: Some(0),
                media_rate: 1.0,
            },
        ];
    }
    Ok(TrackConfig {
        kind: track.kind,
        timescale: track.timescale,
        language: track.language.clone(),
        handler_name: track.handler_name.clone(),
        codec: track.codec.clone(),
        width: track.width,
        height: track.height,
        edit_list: edits,
    })
}

fn remux(
    video: &Path,
    audio: &Path,
    output: &Path,
    stop: &AtomicBool,
    bytes: &AtomicU64,
) -> Result<()> {
    let partial = output.with_extension(format!("{}.partial", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut inputs = [open(video)?, open(audio)?];
        let mut tracks = [
            inputs[0]
                .tracks()
                .iter()
                .find(|t| t.kind == TrackKind::Video)
                .context("Saved MP4 has no video track")?
                .clone(),
            inputs[1]
                .tracks()
                .iter()
                .find(|t| t.kind == TrackKind::Audio)
                .context("Saved MP4 has no audio track")?
                .clone(),
        ];
        for i in 0..2 {
            tracks[i].sample_count = u32::try_from(inputs[i].samples(tracks[i].id)?.len())
                .context("Too many MP4 samples")?;
            tracks[i].duration =
                inputs[i]
                    .samples(tracks[i].id)?
                    .try_fold(0_u64, |sum, sample| {
                        sum.checked_add(u64::from(sample.duration))
                            .context("MP4 duration overflow")
                    })?;
        }
        anyhow::ensure!(
            tracks.iter().all(|t| t.sample_count > 0),
            "Saved MP4 track has no samples"
        );
        let movie_clock = inputs[0].movie_timescale();
        let mut writer = Mp4Writer::new(
            BufWriter::with_capacity(
                64 * 1024,
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&partial)?,
            ),
            WriterConfig {
                movie_timescale: movie_clock,
                // Moving moov to the front can push a chunk past the 4 GB boundary.
                chunk_offsets: ChunkOffsetChoice::Co64,
                ..WriterConfig::default()
            },
        )?;
        let mut ids = Vec::new();
        let mut next = [
            Some(inputs[0].sample_info(tracks[0].id, 0)?),
            Some(inputs[1].sample_info(tracks[1].id, 0)?),
        ];
        let mut expected = [next[0].unwrap().dts, next[1].unwrap().dts];
        for i in 0..2 {
            ids.push(writer.add_track(config(
                &tracks[i],
                inputs[i].movie_timescale(),
                movie_clock,
                expected[i],
            )?)?);
        }
        let mut buffer = Vec::new();
        while next.iter().any(Option::is_some) {
            anyhow::ensure!(!stop.load(Ordering::Relaxed), "MP4 combining paused");
            let i = match (next[0], next[1]) {
                (Some(v), Some(a)) => usize::from(
                    u128::from(v.dts) * u128::from(tracks[1].timescale)
                        > u128::from(a.dts) * u128::from(tracks[0].timescale),
                ),
                (Some(_), None) => 0,
                _ => 1,
            };
            let info = next[i].unwrap();
            // Never silently compress a timestamp gap or discard codec changes.
            anyhow::ensure!(
                info.dts == expected[i] && info.duration > 0,
                "Saved MP4 contains a discontinuous timeline"
            );
            anyhow::ensure!(
                info.description_index == 1,
                "Saved MP4 changes codec configuration mid-track"
            );
            inputs[i].read_sample_into(tracks[i].id, info.index, &mut buffer)?;
            writer.write_sample(
                ids[i],
                &SampleInput {
                    data: &buffer,
                    duration: info.duration,
                    cts_offset: i32::try_from(info.cts_offset)
                        .context("MP4 composition offset overflow")?,
                    is_sync: info.is_sync,
                },
            )?;
            bytes.fetch_add(buffer.len() as u64, Ordering::Relaxed);
            expected[i] = info
                .dts
                .checked_add(u64::from(info.duration))
                .context("MP4 timeline overflow")?;
            next[i] = if info.index + 1 < tracks[i].sample_count {
                Some(inputs[i].sample_info(tracks[i].id, info.index + 1)?)
            } else {
                None
            };
        }
        let mut file = writer
            .finish()?
            .into_inner()
            .map_err(|e| anyhow::anyhow!("Could not flush combined MP4: {e}"))?;
        crate::mp4_layout::fast_start(&mut file, stop)?;
        file.sync_all()?;
        drop(file);
        anyhow::ensure!(!stop.load(Ordering::Relaxed), "MP4 combining paused");
        let combined = open(&partial)?;
        anyhow::ensure!(
            combined.tracks().len() == 2
                && combined
                    .tracks()
                    .iter()
                    .zip(&tracks)
                    .all(|(out, input)| out.sample_count == input.sample_count),
            "Combined MP4 verification failed"
        );
        drop(combined);
        std::fs::rename(&partial, output)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn verify(video: &Path, audio: &Path, output: &Path) {
        remux(
            video,
            audio,
            output,
            &AtomicBool::new(false),
            &AtomicU64::new(0),
        )
        .unwrap();
        use std::io::{Read, Seek, SeekFrom};
        let mut file = File::open(output).unwrap();
        let mut header = [0; 8];
        file.read_exact(&mut header).unwrap();
        let first_size = u32::from_be_bytes(header[..4].try_into().unwrap());
        assert_eq!(&header[4..8], b"ftyp");
        file.seek(SeekFrom::Start(u64::from(first_size))).unwrap();
        file.read_exact(&mut header).unwrap();
        assert_eq!(&header[4..8], b"moov");
        let mut joined = open(output).unwrap();
        assert!(!joined.is_fragmented());
        assert_eq!(joined.tracks().len(), 2);
        for (i, source) in [video, audio].into_iter().enumerate() {
            let mut original = open(source).unwrap();
            let mut before = original.tracks()[0].clone();
            before.sample_count = original.samples(before.id).unwrap().len() as u32;
            let after = joined.tracks()[i].clone();
            assert_eq!(before.codec, after.codec);
            assert_eq!(before.timescale, after.timescale);
            assert_eq!(before.sample_count, after.sample_count);
            let mut expected_edits = before.edit_list.clone();
            for edit in &mut expected_edits {
                edit.segment_duration = scale(
                    edit.segment_duration,
                    original.movie_timescale(),
                    joined.movie_timescale(),
                )
                .unwrap();
            }
            assert_eq!(expected_edits, after.edit_list);
            for index in 0..before.sample_count {
                let a = original.read_sample(before.id, index).unwrap();
                let b = joined.read_sample(after.id, index).unwrap();
                assert_eq!(a.data, b.data, "encoded sample changed at {index}");
                assert_eq!(
                    (a.info.dts, a.info.pts, a.info.duration, a.info.is_sync),
                    (b.info.dts, b.info.pts, b.info.duration, b.info.is_sync),
                    "sample timing changed at {index}"
                );
            }
        }
    }
    #[test]
    fn replaces_embedded_audio_when_reusing_a_combined_video_file() {
        let dir = tempfile::tempdir().unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let video = fixtures.join("sync-bframes-video.mp4");
        let wrong = fixtures.join("alternate-audio.m4a");
        let original = fixtures.join("sync-audio.m4a");
        assert_ne!(
            std::fs::read(&wrong).unwrap(),
            std::fs::read(&original).unwrap()
        );
        let combined = dir.path().join("cached-with-wrong-audio.mp4");
        remux(
            &video,
            &wrong,
            &combined,
            &AtomicBool::new(false),
            &AtomicU64::new(0),
        )
        .unwrap();
        verify(&combined, &original, &dir.path().join("repaired.mp4"));
    }

    #[test]
    fn preserves_fragmented_av1_bframes_aac_priming_and_every_sample_timestamp() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let temp = tempfile::tempdir().unwrap();
        for video in ["sync-fragmented-video.mp4", "sync-bframes-video.mp4"] {
            verify(
                &fixtures.join(video),
                &fixtures.join("sync-audio.m4a"),
                &temp.path().join(video),
            );
            if let Ok(destination) = std::env::var("ZIMSTUDIO_SYNC_OUTPUT_DIR") {
                let destination = PathBuf::from(destination);
                std::fs::create_dir_all(&destination).unwrap();
                std::fs::copy(temp.path().join(video), destination.join(video)).unwrap();
                let config = crate::NewJob {
                    url: "https://example.com/".into(),
                    title: "Playback fixture".into(),
                    kind: crate::SourceKind::Media,
                    max_items: 1,
                    max_depth: 0,
                    delay_ms: 1000,
                    max_resource_mb: 2,
                    language: "eng".into(),
                    description: "Generated sync test".into(),
                    creator: "Test".into(),
                    include_assets: false,
                    capture: Default::default(),
                };
                let mut entry = crate::native_media::entry(
                    "O6VZaBM2Zr4",
                    "Sync test: flash and beep".into(),
                    Some(4.),
                    "Synthetic video with a flash and beep every second".into(),
                    false,
                );
                entry.media_file = Some(video.into());
                std::fs::write(
                    destination.join(format!("{video}.html")),
                    crate::templates::render_media(
                        crate::ArchiveTemplate::ScreeningRoom,
                        &config,
                        &entry,
                    ),
                )
                .unwrap();
                let mut catalog = crate::templates::SearchCatalog::default();
                catalog.add(&entry, &config.kind);
                catalog.documents[0].url = format!("{video}.html");
                for (suffix, label) in [("second", "Second video"), ("third", "Sync finale")] {
                    let mut next_entry = entry.clone();
                    next_entry.id = format!("{}-{suffix}", entry.id);
                    next_entry.title = label.into();
                    next_entry.text = "Another synthetic test clip".into();
                    next_entry.media_file = Some(format!("{suffix}-{video}"));
                    std::fs::copy(
                        temp.path().join(video),
                        destination.join(next_entry.media_file.as_ref().unwrap()),
                    )
                    .unwrap();
                    catalog.add(&next_entry, &config.kind);
                    catalog.documents.last_mut().unwrap().url = format!("{suffix}-{video}.html");
                    std::fs::write(
                        destination.join(format!("{suffix}-{video}.html")),
                        crate::templates::render_media(
                            crate::ArchiveTemplate::ScreeningRoom,
                            &config,
                            &next_entry,
                        ),
                    )
                    .unwrap();
                }
                std::fs::write(
                    destination.join(format!("{video}.index.html")),
                    crate::templates::render_index(
                        crate::ArchiveTemplate::ScreeningRoom,
                        &config,
                        &catalog,
                        false,
                    )
                    .unwrap(),
                )
                .unwrap();
                std::fs::write(destination.join("gallery.js"), crate::templates::GALLERY_JS)
                    .unwrap();
                std::fs::write(destination.join("search.js"), crate::templates::SEARCH_JS).unwrap();
                for (file, data) in [
                    ("studio.css", crate::templates::CSS.as_bytes()),
                    ("favicon.png", crate::templates::ICON),
                    ("archive-sans.woff2", crate::templates::SANS),
                    ("archive-display.woff2", crate::templates::DISPLAY),
                ] {
                    std::fs::write(destination.join(file), data).unwrap();
                }
            }
        }
    }
    #[test]
    fn cancellation_or_invalid_input_leaves_existing_output_untouched() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("combined.mp4");
        std::fs::write(&output, b"previous completed output").unwrap();
        assert!(remux(
            &fixtures.join("sync-fragmented-video.mp4"),
            &fixtures.join("sync-audio.m4a"),
            &output,
            &AtomicBool::new(true),
            &AtomicU64::new(0)
        )
        .is_err());
        assert_eq!(
            std::fs::read(&output).unwrap(),
            b"previous completed output"
        );
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
        assert!(remux(
            &output,
            &output,
            &temp.path().join("invalid.mp4"),
            &AtomicBool::new(false),
            &AtomicU64::new(0)
        )
        .is_err());
        assert!(!temp.path().join("invalid.mp4").exists());
    }
    #[test]
    #[ignore = "Requires saved provider tracks in ZIMSTUDIO_SYNC_FIXTURE_DIR; no network requests"]
    fn combines_saved_provider_tracks_with_identical_payloads_and_timing() {
        let dir = PathBuf::from(
            std::env::var("ZIMSTUDIO_SYNC_FIXTURE_DIR")
                .expect("Set the folder containing video.mp4 and audio.m4a"),
        );
        verify(
            &dir.join("video.mp4"),
            &dir.join("audio.m4a"),
            &dir.join("combined.mp4"),
        );
    }
}
