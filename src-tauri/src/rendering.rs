use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Semaphore;
use zim_engine::render::{PageRenderer, RenderFuture, RenderRequest, RenderedPage, CAPTURE_SCRIPT};

pub struct BrowserRenderer {
    app: tauri::AppHandle,
    gate: Arc<Semaphore>,
}
impl BrowserRenderer {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self {
            app,
            gate: Arc::new(Semaphore::new(1)),
        }
    }
}

struct Cleanup {
    app: tauri::AppHandle,
    id: String,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let app = self.app.clone();
        let id = self.id.clone();
        #[cfg(any(target_os = "ios", target_os = "android"))]
        tauri::async_runtime::spawn_blocking(move || {
            let _ = app
                .state::<tauri_plugin_mobile_files::MobileFiles<tauri::Wry>>()
                .cancel_render(&id);
        });
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        if let Some(window) = app.get_webview_window(&format!("capture-{id}")) {
            let _ = window.close();
        }
    }
}

impl PageRenderer for BrowserRenderer {
    fn render(&self, request: RenderRequest) -> RenderFuture {
        let app = self.app.clone();
        let gate = self.gate.clone();
        Box::pin(async move {
            let _permit = gate
                .acquire_owned()
                .await
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            if let Some(events) = &request.events {
                let _ = events.send(zim_engine::render::RenderEvent::Started);
            }
            let _cleanup = Cleanup {
                app: app.clone(),
                id: request.id.clone(),
            };
            #[cfg(any(target_os = "ios", target_os = "android"))]
            {
                let value = serde_json::to_value(&request)?;
                let render_app = app.clone();
                let mut completion = tauri::async_runtime::spawn_blocking(move || {
                    render_app
                        .state::<tauri_plugin_mobile_files::MobileFiles<tauri::Wry>>()
                        .render_page(value, CAPTURE_SCRIPT)
                });
                let result = loop {
                    tokio::select! {
                        result = &mut completion => break result?.map_err(std::io::Error::other)?,
                        _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                            let status_app = app.clone(); let id = request.id.clone();
                            let status = tauri::async_runtime::spawn_blocking(move || status_app.state::<tauri_plugin_mobile_files::MobileFiles<tauri::Wry>>().render_status(&id));
                            if let Ok(Ok(Ok(value))) = tokio::time::timeout(std::time::Duration::from_secs(2),status).await {
                                if let (Some(events),Some(value)) = (&request.events,value.get("progress")) {
                                    if let Ok(progress) = serde_json::from_value(value.clone()) { let _=events.send(zim_engine::render::RenderEvent::Progress(progress)); }
                                }
                            }
                        }
                    }
                };
                Ok(serde_json::from_value::<RenderedPage>(result)?)
            }
            #[cfg(not(any(target_os = "ios", target_os = "android")))]
            {
                desktop(&app, request).await
            }
        })
    }
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
async fn desktop(
    app: &tauri::AppHandle,
    request: RenderRequest,
) -> Result<RenderedPage, anyhow::Error> {
    use tauri::webview::NewWindowResponse;
    let label = format!("capture-{}", request.id);
    let script = format!(
        "window.__zimsmithRenderOptions={};\n{}",
        serde_json::to_string(&request)?,
        CAPTURE_SCRIPT
    );
    let scopes: Vec<_> = request
        .scopes
        .iter()
        .filter_map(|s| zim_engine::web::http_url(s).ok())
        .collect();
    let initial = zim_engine::web::http_url(&request.url)?;
    let initial_for_nav = initial.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let app_for_build = app.clone();
    app.run_on_main_thread(move || {
        let result = tauri::WebviewWindowBuilder::new(
            &app_for_build,
            label,
            tauri::WebviewUrl::External(initial),
        )
        .title("Zimsmith page capture")
        .inner_size(1024., 768.)
        .visible(false)
        .incognito(true)
        .initialization_script(script)
        .on_navigation(move |url| {
            url == &initial_for_nav
                || scopes
                    .iter()
                    .any(|root| zim_engine::web::in_scope(root, url))
        })
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .on_download(|_, _| false)
        .on_permission_request(|_, _| tauri::webview::PermissionResponse::Deny)
        .build()
        .map_err(|e| std::io::Error::other(e.to_string()));
        let _ = tx.send(result);
    })?;
    let window = rx.await??;
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(request.timeout_secs + 5);
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx = std::sync::Mutex::new(Some(tx));
        window.eval_with_callback("(()=>{try{return window.__zimsmithCapture?.poll()||null}catch(e){return {error:String(e)}}})()",move |value| { if let Some(tx)=tx.lock().unwrap().take() { let _=tx.send(value); } })?;
        let Ok(Ok(value)) = tokio::time::timeout(std::time::Duration::from_secs(3), rx).await
        else {
            continue;
        };
        let mut value: serde_json::Value = serde_json::from_str(&value).unwrap_or_default();
        if let Some(s) = value.as_str() {
            value = serde_json::from_str(s).unwrap_or_default();
        }
        if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
            return Err(std::io::Error::other(error.to_string()).into());
        }
        if let (Some(events), Some(value)) = (&request.events, value.get("progress")) {
            if let Ok(progress) = serde_json::from_value(value.clone()) {
                let _ = events.send(zim_engine::render::RenderEvent::Progress(progress));
            }
        }
        if value.get("html").is_some() {
            return Ok(serde_json::from_value(value)?);
        }
    }
    Err(std::io::Error::other("Browser rendering timed out. Increase its timeout or retry.").into())
}
