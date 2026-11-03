These synthetic four-second fixtures contain a white flash and an 880 Hz beep at each whole second. They were generated for this repository; they contain no third-party media.

- `sync-fragmented-video.mp4`: AV1 in fragmented MP4, matching the container/codec combination returned for the SNL download.
- `sync-bframes-video.mp4`: H.264 with B-frames and an edit list.
- `sync-audio.m4a`: AAC with encoder priming recorded in its edit list.

Generated with FFmpeg (only needed to regenerate fixtures, never to run the app):

```sh
ffmpeg -f lavfi -i 'color=c=black:s=160x96:r=30:d=4' -vf "drawbox=color=white:t=fill:enable='lt(mod(t,1),0.1)'" -c:v libsvtav1 -preset 12 -crf 45 -g 30 -an -movflags empty_moov+frag_keyframe+default_base_moof sync-fragmented-video.mp4
ffmpeg -f lavfi -i 'color=c=black:s=160x96:r=30:d=4' -vf "drawbox=color=white:t=fill:enable='lt(mod(t,1),0.1)'" -c:v libx264 -bf 3 -g 30 -an sync-bframes-video.mp4
ffmpeg -f lavfi -i "aevalsrc='if(lt(mod(t,1),0.1),sin(2*PI*880*t),0)':s=48000:d=4" -c:a aac -b:a 32k sync-audio.m4a
```

Rust tests combine both video variants with the audio and compare every encoded sample, DTS/PTS, duration, sync flag, codec configuration, and edit list (with movie-timescale conversion). Where the host supports the codec, a WebKit test opens the actual exported player HTML and checks its single audio/video timeline through playback, pause, seeking, and rate changes.
