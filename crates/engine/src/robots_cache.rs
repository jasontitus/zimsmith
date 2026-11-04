use crate::{model::now, web::key, Record};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(crate) struct Checkpoint {
    pub origin: String,
    pub valid_until: u64,
    pub text: Option<String>,
    pub delay_ms: u64,
    pub error: Option<String>,
    pub attempts: u8,
    #[serde(default)]
    pub passive_fallback: bool,
}

pub(crate) async fn load(record: &Record, origin: &str) -> Option<Checkpoint> {
    let path = record
        .dir
        .join("robots")
        .join(format!("{}.json", key(origin)));
    if tokio::fs::metadata(&path).await.ok()?.len() > 8 * 1024 * 1024 {
        return None;
    }
    let bytes = tokio::fs::read(path).await.ok()?;
    let value: Checkpoint = serde_json::from_slice(&bytes).ok()?;
    (value.origin == origin && value.valid_until > now()).then_some(value)
}

pub(crate) async fn save(record: &Record, value: &Checkpoint) -> Result<()> {
    let directory = record.dir.join("robots");
    tokio::fs::create_dir_all(&directory).await?;
    let path = directory.join(format!("{}.json", key(&value.origin)));
    let bytes = serde_json::to_vec(value)?;
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        use std::io::Write;
        let mut file = atomic_write_file::AtomicWriteFile::open(path)?;
        file.write_all(&bytes)?;
        file.commit()
    })
    .await??;
    Ok(())
}
