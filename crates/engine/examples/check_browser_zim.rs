//! Independent read-back for browser-writer fixtures, using the native zimru.
fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .expect("Pass the browser-generated ZIM path");
    let archive = zimru::Archive::open(path)?;
    anyhow::ensure!(archive.check()?, "Checksum mismatch");
    anyhow::ensure!(
        archive.check_dirent_ptrs()?
            && archive.check_dirent_order()?
            && archive.check_title_index()?
            && archive.check_cluster_ptrs()?
            && archive.check_mimetypes()?,
        "ZIM structure is invalid"
    );
    anyhow::ensure!(
        !archive.get_text(&archive.main_path()?)?.is_empty(),
        "Missing starting page"
    );
    let index = archive.get_text("index.html")?;
    anyhow::ensure!(
        index.contains("catalog") && index.contains("search.js"),
        "Missing search catalog"
    );
    for name in [
        "Title",
        "Description",
        "Creator",
        "Publisher",
        "Language",
        "Date",
        "Illustration_48x48@1",
    ] {
        anyhow::ensure!(
            !archive.get_metadata(name)?.is_empty(),
            "Missing metadata: {name}"
        );
    }
    let expected = std::env::args().nth(2).map(std::fs::read).transpose()?;
    let mut media_found = false;
    for entry in archive.iter_by_path() {
        let entry = entry?;
        let payload = entry.get_item(false)?.bytes()?;
        if entry.path().starts_with("media-") {
            if let Some(expected) = &expected {
                anyhow::ensure!(
                    payload.to_vec() == *expected,
                    "Resumed media differs from the source bytes"
                );
                media_found = true;
            }
        }
    }
    anyhow::ensure!(
        expected.is_none() || media_found,
        "Expected media payload is missing"
    );
    println!("Verified browser ZIM: {} entries, valid tables, checksum, metadata, contents, and readable payloads.", archive.all_entry_count());
    Ok(())
}
