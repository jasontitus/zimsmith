//! Verify failed asset URLs from a saved manifest without changing the original crawl.
use anyhow::{Context, Result};
use std::path::PathBuf;
use zim_engine::{web, Job, Record};

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let manifest = PathBuf::from(
        args.next()
            .context("Pass a saved job.json and a fresh private output directory")?,
    );
    let dir = PathBuf::from(
        args.next()
            .context("Pass a fresh private output directory")?,
    );
    anyhow::ensure!(
        !dir.join("job.json").exists(),
        "Output already contains a crawl; choose a fresh directory"
    );
    tokio::fs::create_dir_all(dir.join("cache")).await?;
    let mut job: Job = serde_json::from_slice(&tokio::fs::read(manifest).await?)?;
    job.config.validate()?;
    let urls: Vec<_> = job
        .resources
        .values()
        .filter(|resource| resource.state == "failed")
        .map(|resource| resource.url.clone())
        .collect();
    job.logs.clear();
    let record = Record {
        renderer: Default::default(),
        job: tokio::sync::Mutex::new(job),
        stop: Default::default(),
        running: Default::default(),
        import_cancel: Default::default(),
        active_asset_requests: Default::default(),
        peak_asset_requests: Default::default(),
        dir: dir.clone(),
    };
    let network = web::Network::new(&record).await?;
    let mut succeeded = 0;
    let mut megabytes = 0.0;
    let mut failures = Vec::new();
    for url in urls {
        match network.get_asset(&web::http_url(&url)?).await {
            Ok((_, _, body)) => {
                succeeded += 1;
                megabytes += body.len() as f64 / 1_000_000.0;
            }
            Err(error) => {
                failures.push(serde_json::json!({"url":url,"error":format!("{error:#}")}))
            }
        }
    }
    let report = serde_json::json!({"succeeded":succeeded,"failed":failures.len(),"megabytes":megabytes,"failures":failures});
    tokio::fs::write(dir.join("result.json"), serde_json::to_vec_pretty(&report)?).await?;
    println!(
        "{succeeded} assets succeeded; {} failed; {megabytes:.2} MB verified",
        failures.len()
    );
    anyhow::ensure!(
        failures.is_empty(),
        "Some assets remain unavailable; see the private result.json"
    );
    Ok(())
}
