//! Original-audio selection for YouTube. Archive language is metadata, not a dub preference.
use anyhow::Result;
use rustypipe::model::{
    AudioCodec, AudioStream, AudioTrackType, VideoCodec, VideoPlayer, VideoStream,
};
use std::{cmp::Reverse, collections::BTreeSet};

#[derive(Debug)]
pub struct OriginalAudioUnavailable(pub String);
impl std::fmt::Display for OriginalAudioUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for OriginalAudioUnavailable {}

fn original(audio: &AudioStream) -> bool {
    audio.track.as_ref().is_some_and(|track| {
        track.track_type == Some(AudioTrackType::Original)
            || (track.track_type.is_none()
                && track
                    .lang_name
                    .to_ascii_lowercase()
                    .split(|c: char| !c.is_ascii_alphabetic())
                    .any(|word| word == "original"))
    })
}
fn translated_or_descriptive(audio: &AudioStream) -> bool {
    audio.track.as_ref().is_some_and(|track| {
        matches!(
            track.track_type,
            Some(AudioTrackType::Dubbed | AudioTrackType::DubbedAuto | AudioTrackType::Descriptive)
        )
    })
}

pub fn select_audio(streams: &[AudioStream]) -> Result<&AudioStream> {
    let clear: Vec<_> = streams
        .iter()
        .filter(|s| s.drm_track_type.is_none())
        .collect();
    let has_original = clear.iter().any(|s| original(s));
    if !has_original {
        let ids: BTreeSet<_> = clear
            .iter()
            .filter_map(|s| s.track.as_ref().map(|t| t.id.as_str()))
            .collect();
        if ids.len() > 1 || clear.iter().any(|s| translated_or_descriptive(s)) {
            return Err(OriginalAudioUnavailable("The provider did not identify the original audio among multiple languages; refusing to choose a dub".into()).into());
        }
    }
    clear
        .into_iter()
        .filter(|s| !has_original || original(s))
        // The local muxer consumes MP4/AAC. Never substitute a different
        // language merely because its codec or bitrate is preferable.
        .filter(|s| s.codec == AudioCodec::Mp4a && s.mime.contains("mp4"))
        .max_by_key(|s| {
            (
                s.track.as_ref().is_some_and(|t| t.is_default),
                s.bitrate,
                Reverse(s.track.as_ref().map(|t| t.id.as_str()).unwrap_or("")),
                Reverse(s.itag),
            )
        })
        .ok_or_else(|| {
            OriginalAudioUnavailable("No playable MP4/AAC original audio track was provided".into())
                .into()
        })
}

pub fn audio_identity(video_id: &str, audio: &AudioStream) -> String {
    let track = audio.track.as_ref().map(|t| (&t.id, &t.lang, t.track_type));
    format!(
        "youtube-audio-v2:{video_id}:{}:{}",
        audio.itag,
        serde_json::to_string(&track).expect("audio track metadata")
    )
}

pub struct Selection<'a> {
    pub video: &'a VideoStream,
    pub audio: Option<&'a AudioStream>,
}
impl Selection<'_> {
    pub fn audio_note(&self) -> String {
        match self.audio.and_then(|a| a.track.as_ref()) {
            Some(track) => format!(
                "Audio: {} · {} · {} (track {})",
                track.lang_name,
                track.lang.as_deref().unwrap_or("language not reported"),
                if self.audio.is_some_and(original) {
                    "original"
                } else {
                    "provider's only track; original marker unavailable"
                },
                track.id
            ),
            None => "Audio: provider's single track; language metadata unavailable".into(),
        }
    }
}

pub fn select(player: &VideoPlayer) -> Result<Selection<'_>> {
    select_with_height(player, 720)
}

pub fn select_with_height(player: &VideoPlayer, max_height: u32) -> Result<Selection<'_>> {
    anyhow::ensure!(
        player.drm.is_none() && !player.details.is_live,
        "Protected and live streams are not supported"
    );
    let playable = |s: &&VideoStream| {
        s.drm_track_type.is_none()
            && s.height <= max_height
            && s.mime.contains("mp4")
            && s.codec == VideoCodec::Avc1
    };
    let combined = player
        .video_streams
        .iter()
        .filter(playable)
        .max_by_key(|s| (s.height, s.fps));
    let separate = player
        .video_only_streams
        .iter()
        .filter(playable)
        .max_by_key(|s| (s.height, s.fps));
    let named_audio = player
        .audio_streams
        .iter()
        .any(|s| s.drm_track_type.is_none() && s.track.is_some());
    if !named_audio
        && combined.is_some_and(|c| {
            player.audio_streams.is_empty() || separate.is_none_or(|s| c.height >= s.height)
        })
    {
        return Ok(Selection {
            video: combined.unwrap(),
            audio: None,
        });
    }
    let audio = select_audio(&player.audio_streams)?;
    // Prefer a combined stream on equal resolution; its embedded audio is
    // discarded when an explicit original audio track is provided.
    let video = match (combined, separate) {
        (Some(c), Some(s)) => if c.height >= s.height { c } else { s },
        (Some(c), None) => c,
        (None, Some(s)) => s,
        _ => anyhow::bail!("No compatible MP4/H.264 video is available at or below {max_height}p. Choose a higher quality preset."),
    };
    Ok(Selection {
        video,
        audio: Some(audio),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn audio(
        id: &str,
        kind: Option<&str>,
        default: bool,
        bitrate: u32,
        codec: &str,
    ) -> AudioStream {
        serde_json::from_value(json!({"url":"https://media.example/audio", "itag":140,"bitrate":bitrate,"average_bitrate":bitrate,"size":1000,
            "index_range":null,"init_range":null,"duration_ms":1000,"mime":if codec=="mp4a" {"audio/mp4"}else{"audio/webm"},
            "format":if codec=="mp4a" {"m4a"}else{"webm"},"codec":codec,"channels":2,"loudness_db":null,
            "track":{"id":id,"lang":id.split('.').next(),"lang_name":id,"is_default":default,"track_type":kind},"drm_track_type":null,"drm_systems":[]})).unwrap()
    }
    fn player(audio: Vec<AudioStream>, combined: bool) -> VideoPlayer {
        let video = json!({"url":"https://media.example/video","itag":18,"bitrate":1000,"average_bitrate":1000,"size":1000,"index_range":null,"init_range":null,"duration_ms":1000,"width":640,"height":360,"fps":30,"quality":"360p","hdr":false,"mime":"video/mp4","format":"mp4","codec":"avc1","drm_track_type":null,"drm_systems":[]});
        serde_json::from_value(json!({"details":{"id":"fixture1234","name":"Fixture","description":null,"duration":1,"thumbnail":[],"channel_id":"fixture","channel_name":null,"view_count":null,"keywords":[],"is_live":false,"is_live_content":false},
            "video_streams":if combined {vec![video.clone()]}else{vec![]},"video_only_streams":[video],"audio_streams":audio,"subtitles":[],"expires_in_seconds":1000,"valid_until":"2030-01-01T00:00:00Z","hls_manifest_url":null,"dash_manifest_url":null,"preview_frames":[],"drm":null,"client_type":"vision_os","visitor_data":null})).unwrap()
    }
    #[test]
    fn quality_caps_choose_the_best_compatible_video_and_keep_original_audio() {
        let mut p = player(
            vec![
                audio("en.4", Some("original"), false, 128000, "mp4a"),
                audio("ja.10", Some("dubbed-auto"), true, 140000, "mp4a"),
            ],
            true,
        );
        let mut hd = p.video_only_streams[0].clone();
        hd.height = 720;
        hd.itag = 136;
        let mut full_hd = hd.clone();
        full_hd.height = 1080;
        full_hd.itag = 137;
        p.video_only_streams.extend([hd, full_hd]);
        for cap in [360, 720, 1080] {
            let selected = select_with_height(&p, cap).unwrap();
            assert_eq!(selected.video.height, cap);
            assert_eq!(selected.audio.unwrap().track.as_ref().unwrap().id, "en.4");
        }
        assert!(select_with_height(&p, 144).is_err());
    }

    #[test]
    fn original_wins_over_higher_bitrate_dubs_defaults_and_order() {
        let streams = vec![
            audio("en-US.4", Some("original"), false, 128000, "mp4a"),
            audio("ja.10", Some("dubbed-auto"), true, 130699, "mp4a"),
            audio("ru.10", Some("dubbed-auto"), false, 140000, "mp4a"),
        ];
        for order in [streams.clone(), streams.into_iter().rev().collect()] {
            assert_eq!(
                select_audio(&order).unwrap().track.as_ref().unwrap().id,
                "en-US.4"
            );
        }
        let streams = [
            audio("ja.4", Some("original"), false, 90000, "mp4a"),
            audio("en.10", Some("dubbed-auto"), true, 130000, "mp4a"),
        ];
        assert_eq!(
            select_audio(&streams).unwrap().track.as_ref().unwrap().id,
            "ja.4",
            "Original does not mean English"
        );
    }
    #[test]
    fn never_uses_a_dub_to_work_around_missing_original_or_codec_support() {
        assert!(
            select_audio(&[audio("ja.10", Some("dubbed-auto"), true, 130699, "mp4a")]).is_err()
        );
        assert!(select_audio(&[
            audio("en.4", Some("original"), false, 128000, "opus"),
            audio("ja.10", Some("dubbed-auto"), true, 130699, "mp4a")
        ])
        .is_err());
        assert!(select_audio(&[
            audio("en.0", None, true, 128000, "mp4a"),
            audio("ru.0", None, false, 130000, "mp4a")
        ])
        .is_err());
        let mut single = audio("en.0", None, true, 128000, "mp4a");
        single.track = None;
        assert!(select_audio(&[single]).is_ok());
    }
    #[test]
    fn combined_video_still_gets_the_selected_original_audio() {
        let p = player(
            vec![
                audio("en.4", Some("original"), false, 128000, "mp4a"),
                audio("ja.10", Some("dubbed-auto"), true, 130699, "mp4a"),
            ],
            true,
        );
        let selected = select(&p).unwrap();
        assert!(std::ptr::eq(selected.video, &p.video_streams[0]));
        assert_eq!(selected.audio.unwrap().track.as_ref().unwrap().id, "en.4");
        assert!(selected.audio_note().contains("original"));
    }
    #[test]
    fn track_identity_prevents_reusing_same_itag_in_another_language() {
        let en = audio("en.4", Some("original"), false, 128000, "mp4a");
        let ja = audio("ja.10", Some("dubbed-auto"), true, 128000, "mp4a");
        assert_ne!(
            audio_identity("fixture1234", &en),
            audio_identity("fixture1234", &ja)
        );
        assert_ne!(audio_identity("fixture1234", &en), "fixture1234:140");
        assert_eq!(
            audio_identity("fixture1234", &en),
            audio_identity("fixture1234", &en)
        );
    }
}
