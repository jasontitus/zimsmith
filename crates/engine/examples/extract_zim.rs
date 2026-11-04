//! Extract content entries into a new directory for local capture inspection.
use anyhow::{Context, Result};
use std::path::{Component, PathBuf};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let source = args
        .next()
        .context("Usage: extract_zim ARCHIVE NEW_DIRECTORY")?;
    let output = PathBuf::from(args.next().context("Pass a new output directory")?);
    anyhow::ensure!(args.next().is_none(), "Unexpected extra arguments");
    let archive = zimru::Archive::open(source)?;
    std::fs::create_dir(&output).context("Output directory must not already exist")?;
    let mut count = 0;
    for entry in archive.iter_by_path() {
        let entry = entry?;
        if entry.namespace() != b'C' {
            continue;
        }
        let relative = PathBuf::from(entry.path());
        anyhow::ensure!(
            !relative.as_os_str().is_empty()
                && relative
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
            "Unsafe archive path"
        );
        let target = output.join(relative);
        std::fs::create_dir_all(target.parent().unwrap())?;
        let bytes = entry.get_item(false)?.bytes()?;
        std::fs::write(target, &bytes[..])?;
        count += 1;
    }
    println!("Extracted {count} content entries.");
    Ok(())
}
