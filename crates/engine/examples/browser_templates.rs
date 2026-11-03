//! Refresh the browser's template shells from the native renderer.
use zim_engine::{templates, NewJob};

fn main() -> anyhow::Result<()> {
    let config: NewJob = serde_json::from_value(serde_json::json!({
        "url":"https://zimsmith.example/__SOURCE__", "title":"__TITLE__",
        "description":"__DESCRIPTION__", "creator":"__CREATOR__", "language":"eng",
        "kind":"website", "max_items":100, "max_depth":2, "delay_ms":2000,
        "max_resource_mb":256, "include_assets":true
    }))?;
    let mut shells = serde_json::Map::new();
    for option in templates::catalog() {
        shells.insert(
            option.id.id().into(),
            serde_json::Value::String(templates::render_index(
                option.id,
                &config,
                &templates::SearchCatalog::default(),
                false,
            )?),
        );
    }
    let output =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/browser/templates.json");
    std::fs::create_dir_all(output.parent().unwrap())?;
    std::fs::write(
        output,
        serde_json::to_string_pretty(
            &serde_json::json!({"catalog":templates::catalog(), "shells":shells}),
        )?,
    )?;
    Ok(())
}
