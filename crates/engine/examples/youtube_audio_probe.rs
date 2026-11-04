//! Inspect provider audio metadata without downloading media or using login cookies.
use anyhow::{Context, Result};
use rustypipe::{
    client::{ClientType, RustyPipe},
    model::AudioCodec,
};
use std::collections::BTreeMap;

#[tokio::main]
async fn main() -> Result<()> {
    let input = std::env::args()
        .nth(1)
        .context("Pass a YouTube video URL")?;
    let url = zim_engine::web::source_url(&input)?;
    let id = if url.host_str() == Some("youtu.be") {
        url.path().trim_start_matches('/').to_owned()
    } else {
        url.query_pairs()
            .find(|(k, _)| k == "v")
            .map(|(_, v)| v.into_owned())
            .context("Pass a video URL with v=VIDEO_ID")?
    };
    anyhow::ensure!(
        id.len() == 11
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')),
        "Invalid video ID"
    );
    let temp = tempfile::tempdir()?;
    let rp = RustyPipe::builder()
        .storage_dir(temp.path())
        .no_reporter()
        .timeout(std::time::Duration::from_secs(45))
        .n_http_retries(1)
        .build()?;
    if std::env::args().any(|arg| arg == "--profiles") {
        for client in [
            ClientType::VisionOs,
            ClientType::Ios,
            ClientType::Android,
            ClientType::Tv,
            ClientType::Desktop,
        ] {
            let result = rp.query().player_from_client(&id, client).await;
            match result {
                Ok(player) => println!(
                    "{}",
                    serde_json::json!({"client":format!("{client:?}"),"title":player.details.name,"audio":player.audio_streams.len(),"video":player.video_streams.len(),"video_only":player.video_only_streams.len(),"selection":zim_engine::youtube_audio::select(&player).map(|s|s.audio_note()).map_err(|e|e.to_string())})
                ),
                Err(error) => println!(
                    "{}",
                    serde_json::json!({"client":format!("{client:?}"),"error":error.to_string()})
                ),
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        return Ok(());
    }
    let player = rp
        .query()
        .player_from_clients(
            &id,
            &[ClientType::VisionOs, ClientType::Ios, ClientType::Android],
        )
        .await?;
    let old = player
        .audio_streams
        .iter()
        .filter(|s| s.drm_track_type.is_none())
        .max_by_key(|s| {
            (
                s.codec == AudioCodec::Mp4a,
                s.mime.contains("mp4"),
                s.bitrate,
            )
        });
    let mut tracks = BTreeMap::new();
    for audio in &player.audio_streams {
        let key = audio
            .track
            .as_ref()
            .map(|t| t.id.as_str())
            .unwrap_or("unlabelled");
        tracks.entry(key).or_insert_with(||serde_json::json!({"track":audio.track,"bitrate":audio.bitrate,"itag":audio.itag,"codec":audio.codec}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"client":format!("{:?}",player.client_type),"title":player.details.name,"selected_audio":zim_engine::youtube_audio::select(&player).map(|s|serde_json::json!({"audio":s.audio.map(|a|&a.track),"note":s.audio_note()})).map_err(|e|e.to_string()),"old_bitrate_choice":old.map(|a|serde_json::json!({"track":a.track,"itag":a.itag,"bitrate":a.bitrate})),"tracks":tracks,"combined_streams":player.video_streams.len(),"mp4_video_only":player.video_only_streams.iter().filter(|v|v.mime.contains("mp4")&&v.drm_track_type.is_none()).count()})
        )?
    );
    Ok(())
}
