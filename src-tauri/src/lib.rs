pub mod rendering;
mod sharing;
mod updates;
use tauri::{Manager, State};
use zim_engine::{templates, ArchiveTemplate, Engine, Job, NewJob, Settings};

#[tauri::command]
async fn refresh_javascript(engine: State<'_, Engine>, id: String) -> Result<Job, String> {
    engine
        .refresh_javascript(&id)
        .await
        .map(Job::view)
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn get_shared_links(_app: tauri::AppHandle) -> Result<Vec<sharing::SharedLink>, String> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    return tauri::async_runtime::spawn_blocking(move || {
        let value = _app
            .state::<tauri_plugin_mobile_files::MobileFiles<tauri::Wry>>()
            .shared_links()?;
        let links: Vec<sharing::IncomingLink> =
            serde_json::from_value(value["items"].clone()).map_err(|e| e.to_string())?;
        Ok(links.into_iter().map(Into::into).collect())
    })
    .await
    .map_err(|e| e.to_string())?;
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    Ok(Vec::new())
}

#[tauri::command]
async fn dismiss_shared_link(_app: tauri::AppHandle, id: String) -> Result<(), String> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    return tauri::async_runtime::spawn_blocking(move || {
        _app.state::<tauri_plugin_mobile_files::MobileFiles<tauri::Wry>>()
            .dismiss_shared_link(&id)
            .map(|_| ())
    })
    .await
    .map_err(|e| e.to_string())?;
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let _ = id;
        Ok(())
    }
}

#[tauri::command]
fn archive_templates() -> Vec<templates::TemplateOption> {
    templates::catalog()
}
#[tauri::command]
fn preview_archive_template(template: ArchiveTemplate) -> Result<String, String> {
    templates::preview(template).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_settings(engine: State<'_, Engine>) -> Result<Settings, String> {
    Ok(engine.settings().await)
}
#[tauri::command]
async fn update_settings(
    engine: State<'_, Engine>,
    settings: Settings,
) -> Result<Settings, String> {
    engine
        .update_settings(settings)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn capabilities(engine: State<'_, Engine>) -> Result<zim_engine::Capabilities, String> {
    let mut capabilities = engine.capabilities().await;
    capabilities.version = env!("CARGO_PKG_VERSION").into();
    Ok(capabilities)
}
#[tauri::command]
async fn list_jobs(engine: State<'_, Engine>) -> Result<Vec<Job>, String> {
    Ok(engine.list().await.into_iter().map(Job::view).collect())
}
#[tauri::command]
async fn create_job(engine: State<'_, Engine>, config: NewJob) -> Result<Job, String> {
    engine
        .create(config)
        .await
        .map(Job::view)
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn get_job(engine: State<'_, Engine>, id: String) -> Result<Job, String> {
    engine
        .get(&id)
        .await
        .map(Job::view)
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn pause_job(engine: State<'_, Engine>, id: String) -> Result<Job, String> {
    engine
        .pause(&id)
        .await
        .map(Job::view)
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn resume_job(engine: State<'_, Engine>, id: String) -> Result<Job, String> {
    engine
        .resume(&id)
        .await
        .map(Job::view)
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn edit_job(engine: State<'_, Engine>, id: String, config: NewJob) -> Result<Job, String> {
    engine
        .edit_retry(&id, config)
        .await
        .map(Job::view)
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn build_job(
    engine: State<'_, Engine>,
    id: String,
    selected: Vec<String>,
) -> Result<Job, String> {
    engine
        .build(&id, selected)
        .await
        .map(Job::view)
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn rebuild_job(
    engine: State<'_, Engine>,
    id: String,
    template: ArchiveTemplate,
) -> Result<Job, String> {
    engine
        .rebuild(&id, template)
        .await
        .map(Job::view)
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn begin_import(
    engine: State<'_, Engine>,
    config: NewJob,
    files: Vec<zim_engine::imports::ImportFile>,
    existing_id: Option<String>,
) -> Result<Job, String> {
    engine
        .begin_import(config, files, existing_id)
        .await
        .map(Job::view)
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn import_offset(engine: State<'_, Engine>, id: String, path: String) -> Result<u64, String> {
    engine
        .import_offset(&id, &path)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn import_chunk(
    engine: State<'_, Engine>,
    id: String,
    path: String,
    offset: u64,
    data: String,
) -> Result<u64, String> {
    engine
        .import_chunk_base64(&id, &path, offset, &data)
        .await
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn finish_import(engine: State<'_, Engine>, id: String) -> Result<Job, String> {
    engine
        .finish_import(&id)
        .await
        .map(Job::view)
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
async fn cancel_import(engine: State<'_, Engine>, id: String) -> Result<(), String> {
    engine
        .cancel_import(&id)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn refresh_job(engine: State<'_, Engine>, id: String) -> Result<Job, String> {
    engine
        .refresh_collection(&id)
        .await
        .map(Job::view)
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn delete_failed_job(engine: State<'_, Engine>, id: String) -> Result<bool, String> {
    engine
        .delete_failed(&id)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn build_available_job(engine: State<'_, Engine>, id: String) -> Result<Job, String> {
    engine
        .build_available(&id)
        .await
        .map(Job::view)
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn repair_youtube_audio(engine: State<'_, Engine>, id: String) -> Result<Job, String> {
    engine
        .repair_youtube_audio(&id)
        .await
        .map(Job::view)
        .map_err(|e| format!("{e:#}"))
}

fn archive_filename(title: &str, id: &str, revision: u64) -> String {
    let name: String = title
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .take(80)
        .collect();
    let name = name.trim();
    let suffix = if revision == 0 {
        String::new()
    } else {
        format!("-r{revision}")
    };
    format!(
        "{}-{}{suffix}.zim",
        if name.is_empty() { "Archive" } else { name },
        &id[..id.len().min(8)]
    )
}

async fn archive_for_handoff(
    app: &tauri::AppHandle,
    engine: &Engine,
    id: &str,
) -> Result<std::path::PathBuf, String> {
    let source = engine.artifact(id).await.map_err(|e| e.to_string())?;
    let job = engine.get(id).await.map_err(|e| e.to_string())?;
    let folder = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("exports")
        .join(id);
    tokio::fs::create_dir_all(&folder)
        .await
        .map_err(|e| e.to_string())?;
    let target = folder.join(archive_filename(
        &job.config.title,
        id,
        job.archive_revision,
    ));
    if !target.exists() {
        // Each revision has a distinct filename. A hard link gives the system picker
        // a friendly name without first duplicating a potentially huge ZIM.
        if tokio::fs::hard_link(&source, &target).await.is_err() {
            copy_archive(&source, &target).await?;
        }
    }
    Ok(target)
}

async fn copy_archive(source: &std::path::Path, target: &std::path::Path) -> Result<(), String> {
    let source = source.to_path_buf();
    let target = target.to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let mut input = std::fs::File::open(source).map_err(|e| e.to_string())?;
        let mut output =
            atomic_write_file::AtomicWriteFile::open(target).map_err(|e| e.to_string())?;
        std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
        output.commit().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
async fn archive_in_downloads(
    app: &tauri::AppHandle,
    source: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let directory = app.path().download_dir().map_err(|e| e.to_string())?;
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|e| e.to_string())?;
    let target = directory.join(source.file_name().ok_or("Archive filename is missing")?);
    if !target.exists() {
        copy_archive(source, &target).await?;
    }
    Ok(target)
}

#[tauri::command]
async fn export_archive(
    app: tauri::AppHandle,
    engine: State<'_, Engine>,
    id: String,
) -> Result<String, String> {
    let source = archive_for_handoff(&app, &engine, &id).await?;
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        let filename = source
            .file_name()
            .ok_or("Archive filename is missing")?
            .to_string_lossy()
            .to_string();
        return tauri::async_runtime::spawn_blocking(move || {
            let result = app
                .state::<tauri_plugin_mobile_files::MobileFiles<tauri::Wry>>()
                .save(&source.display().to_string(), &filename)?;
            Ok(if result["saved"].as_bool().unwrap_or(false) {
                format!("Saved to your chosen folder: {filename}")
            } else {
                "Export canceled".into()
            })
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let target = archive_in_downloads(&app, &source).await?;
        Ok(format!(
            "Saved to Downloads: {}",
            target.file_name().unwrap().to_string_lossy()
        ))
    }
}

#[tauri::command]
async fn open_archive(
    app: tauri::AppHandle,
    engine: State<'_, Engine>,
    id: String,
) -> Result<(), String> {
    let source = archive_for_handoff(&app, &engine, &id).await?;
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        tauri::async_runtime::spawn_blocking(move || {
            app.state::<tauri_plugin_mobile_files::MobileFiles<tauri::Wry>>()
                .open(&source.display().to_string())
                .map(|_| ())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        let target = archive_in_downloads(&app, &source).await?;
        tauri::async_runtime::spawn_blocking(move || {
            open::that(target).map_err(|e| format!("Could not open the archive. Install a ZIM viewer and associate .zim files with it: {e}"))
        }).await.map_err(|e| e.to_string())?
    }
}

#[cfg(all(target_os = "android", feature = "direct-updates"))]
async fn update_identity(app: &tauri::AppHandle) -> Result<updates::Identity, String> {
    let app = app.clone();
    let value = tauri::async_runtime::spawn_blocking(move || {
        app.state::<tauri_plugin_mobile_files::MobileFiles<tauri::Wry>>()
            .app_info()
    })
    .await
    .map_err(|e| e.to_string())??;
    serde_json::from_value(value).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_update_status(_app: tauri::AppHandle) -> Result<updates::Status, String> {
    #[cfg(all(target_os = "android", feature = "direct-updates"))]
    {
        return Ok(_app
            .state::<updates::Updater>()
            .get(&update_identity(&_app).await?)
            .await);
    }
    #[cfg(not(all(target_os = "android", feature = "direct-updates")))]
    Ok(updates::Status::disabled())
}
#[tauri::command]
async fn check_app_updates(
    _app: tauri::AppHandle,
    manual: bool,
) -> Result<updates::Status, String> {
    #[cfg(all(target_os = "android", feature = "direct-updates"))]
    {
        return _app
            .state::<updates::Updater>()
            .check(&update_identity(&_app).await?, manual)
            .await;
    }
    #[cfg(not(all(target_os = "android", feature = "direct-updates")))]
    {
        let _ = manual;
        Ok(updates::Status::disabled())
    }
}
#[tauri::command]
async fn configure_app_updates(
    _app: tauri::AppHandle,
    automatic: bool,
    feed_url: String,
) -> Result<updates::Status, String> {
    #[cfg(all(target_os = "android", feature = "direct-updates"))]
    {
        return _app
            .state::<updates::Updater>()
            .configure(&update_identity(&_app).await?, automatic, feed_url)
            .await;
    }
    #[cfg(not(all(target_os = "android", feature = "direct-updates")))]
    {
        let _ = (automatic, feed_url);
        Err("Direct Android updates are unavailable in this build".into())
    }
}
#[tauri::command]
async fn dismiss_app_update(_app: tauri::AppHandle) -> Result<updates::Status, String> {
    #[cfg(all(target_os = "android", feature = "direct-updates"))]
    {
        return _app
            .state::<updates::Updater>()
            .dismiss(&update_identity(&_app).await?)
            .await;
    }
    #[cfg(not(all(target_os = "android", feature = "direct-updates")))]
    Err("Direct Android updates are unavailable in this build".into())
}
#[tauri::command]
async fn download_app_update(
    _app: tauri::AppHandle,
    _engine: State<'_, Engine>,
) -> Result<(), String> {
    #[cfg(all(target_os = "android", feature = "direct-updates"))]
    {
        if _engine.list().await.iter().any(|j| {
            matches!(
                j.status.as_str(),
                "scanning" | "downloading" | "packaging" | "pausing"
            )
        }) {
            return Err("Pause running jobs before downloading an app update.".into());
        }
        let url = _app
            .state::<updates::Updater>()
            .download_url(&update_identity(&_app).await?)
            .await?;
        return tauri::async_runtime::spawn_blocking(move || {
            _app.state::<tauri_plugin_mobile_files::MobileFiles<tauri::Wry>>()
                .open_web_url(&url)
                .map(|_| ())
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    #[cfg(not(all(target_os = "android", feature = "direct-updates")))]
    Err("Direct Android updates are unavailable in this build".into())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(any(target_os = "ios", target_os = "android"))]
    let builder = builder.plugin(tauri_plugin_mobile_files::init());
    builder
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let engine = tauri::async_runtime::block_on(Engine::open(data.clone()))?;
            tauri::async_runtime::block_on(engine.set_page_renderer(std::sync::Arc::new(
                rendering::BrowserRenderer::new(app.handle().clone()),
            )));
            app.manage(engine);
            #[cfg(all(target_os = "android", feature = "direct-updates"))]
            app.manage(updates::Updater::new(
                data.join("app-updates.json"),
                option_env!("ZIMSMITH_UPDATE_FEED_URL"),
            ));
            Ok(())
        })
        .invoke_handler(|invoke| {
            if invoke.message.webview().label().starts_with("capture-") {
                invoke
                    .resolver
                    .reject("Capture pages cannot invoke application commands");
                return true;
            }
            app_commands()(invoke)
        })
        .run(tauri::generate_context!())
        .expect("Could not start Zim Studio");
}

fn app_commands() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        refresh_javascript,
        get_shared_links,
        dismiss_shared_link,
        get_update_status,
        check_app_updates,
        configure_app_updates,
        dismiss_app_update,
        download_app_update,
        capabilities,
        archive_templates,
        preview_archive_template,
        get_settings,
        update_settings,
        list_jobs,
        create_job,
        get_job,
        pause_job,
        resume_job,
        build_job,
        edit_job,
        rebuild_job,
        repair_youtube_audio,
        build_available_job,
        delete_failed_job,
        refresh_job,
        begin_import,
        import_offset,
        import_chunk,
        finish_import,
        cancel_import,
        export_archive,
        open_archive
    ]
}
