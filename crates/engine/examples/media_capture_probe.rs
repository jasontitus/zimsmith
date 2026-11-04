//! Exercise the real media scan/download/mux/archive pipeline in a supplied private directory.
use anyhow::{Context, Result};
use zim_engine::{capture::VideoQuality, Engine, NewJob, SourceKind};
#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let url = args
        .next()
        .context("Pass a public media URL and a private output directory")?;
    let root = std::path::PathBuf::from(args.next().context("Pass a private output directory")?);
    let engine = Engine::open(&root).await?;
    let mut config = NewJob {
        url,
        title: "Media verification".into(),
        kind: SourceKind::Media,
        max_items: 1,
        max_depth: 0,
        delay_ms: 1000,
        max_resource_mb: 32,
        language: "eng".into(),
        description: "Private media verification".into(),
        creator: "Fixture".into(),
        include_assets: false,
        capture: Default::default(),
    };
    config.capture.subtitles = false;
    config.capture.video_quality = VideoQuality::Small;
    config.capture.max_download_mb = Some(64);
    let initial = engine.create(config).await?;
    for _ in 0..240 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let job = engine.get(&initial.id).await?;
        if ["completed", "failed", "review"].contains(&job.status.as_str()) {
            tokio::fs::write(root.join("result.json"), serde_json::to_vec_pretty(&job)?).await?;
            anyhow::ensure!(
                job.status == "completed",
                "Media check {}: {}",
                job.status,
                job.error.as_deref().unwrap_or("Review needed")
            );
            println!(
                "Media download, mux and ZIM validation completed; {:.2} MB archive",
                job.output_bytes.unwrap_or(0) as f64 / 1_000_000.0
            );
            return Ok(());
        }
    }
    anyhow::bail!("Media verification exceeded its four-minute deadline")
}
