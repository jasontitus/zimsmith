//! Developer smoke test of the real platform webview. Writes only to a supplied private directory.
use tauri::Manager;
use zim_engine::{Engine, NewJob, SourceKind};
fn main() {
    let url = std::env::var("ZIMSMITH_RENDER_PROBE_URL").expect("Set a public test URL");
    let root = std::path::PathBuf::from(
        std::env::var_os("ZIMSMITH_RENDER_PROBE_DIR").expect("Set a private output directory"),
    );
    tauri::Builder::default()
        .setup(move |app| {
            if let Some(main) = app.get_webview_window("main") {
                let _ = main.hide();
            }
            let app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let result: anyhow::Result<serde_json::Value> = async {
                    let engine = Engine::open(&root).await?;
                    engine
                        .set_page_renderer(std::sync::Arc::new(
                            zim_studio_lib::rendering::BrowserRenderer::new(app.clone()),
                        ))
                        .await;
                    let mut config = NewJob {
                        url,
                        title: "Browser capture check".into(),
                        kind: SourceKind::Website,
                        max_items: 10,
                        max_depth: 0,
                        delay_ms: 1000,
                        max_resource_mb: 8,
                        language: "eng".into(),
                        description: "Private rendering verification".into(),
                        creator: "Zimsmith".into(),
                        include_assets: false,
                        capture: Default::default(),
                    };
                    config.capture.render_javascript = true;
                    config.capture.render_timeout_secs = 60;
                    config.capture.embedded_videos = false;
                    let initial = engine.create(config).await?;
                    for _ in 0..180 {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        let job = engine.get(&initial.id).await?;
                        if ["review", "failed", "completed"].contains(&job.status.as_str()) {
                            return Ok(serde_json::to_value(job)?);
                        }
                    }
                    anyhow::bail!("Probe exceeded its timeout")
                }
                .await;
                let (result, code) = match result {
                    Ok(value) => (value, 0),
                    Err(error) => (serde_json::json!({"error":format!("{error:#}")}), 1),
                };
                let _ = std::fs::create_dir_all(&root);
                let _ = std::fs::write(
                    root.join("probe-result.json"),
                    serde_json::to_vec_pretty(&result).unwrap(),
                );
                app.exit(code);
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Could not start rendering smoke test");
}
