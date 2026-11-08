// Offline follow-up for the opt-in live matrix; never resumes network fetching.
use anyhow::{Context, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let root = std::env::args()
        .nth(1)
        .context("Pass the isolated matrix data directory")?;
    let engine = zim_engine::Engine::open(root).await?;
    for job in engine.list().await {
        if !["failed", "paused"].contains(&job.status.as_str()) || job.phase != "download" {
            continue;
        }
        let requests = job.requests;
        engine.build_available(&job.id).await?;
        let done = tokio::time::timeout(std::time::Duration::from_secs(90), async {
            loop {
                let next = engine.get(&job.id).await.unwrap();
                if ["failed", "completed"].contains(&next.status.as_str()) {
                    break next;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await?;
        anyhow::ensure!(
            done.requests == requests,
            "Recovery unexpectedly made network requests"
        );
        println!(
            "{}",
            serde_json::json!({"site":job.config.title,"status":done.status,"zim_mb":done.output_bytes.unwrap_or(0) as f64/1_000_000.0,"validation":done.validation,"report":done.capture_report,"error":done.error})
        );
    }
    Ok(())
}
