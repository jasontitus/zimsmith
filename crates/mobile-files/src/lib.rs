use tauri::{
    Runtime,
    plugin::{Builder, TauriPlugin},
};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_mobile_files);

#[cfg(any(target_os = "android", target_os = "ios"))]
pub struct MobileFiles<R: Runtime>(tauri::plugin::PluginHandle<R>);

#[cfg(any(target_os = "android", target_os = "ios"))]
impl<R: Runtime> MobileFiles<R> {
    pub fn render_page(
        &self,
        request: serde_json::Value,
        script: &str,
    ) -> Result<serde_json::Value, String> {
        self.0
            .run_mobile_plugin(
                "renderPage",
                serde_json::json!({"request":request,"script":script}),
            )
            .map_err(|e| e.to_string())
    }
    pub fn cancel_render(&self, id: &str) -> Result<serde_json::Value, String> {
        self.0
            .run_mobile_plugin("cancelRender", serde_json::json!({"id":id}))
            .map_err(|e| e.to_string())
    }
    pub fn shared_links(&self) -> Result<serde_json::Value, String> {
        self.0
            .run_mobile_plugin("sharedLinks", serde_json::json!({}))
            .map_err(|e| e.to_string())
    }
    pub fn dismiss_shared_link(&self, id: &str) -> Result<serde_json::Value, String> {
        self.0
            .run_mobile_plugin("dismissSharedLink", serde_json::json!({"id":id}))
            .map_err(|e| e.to_string())
    }
    #[cfg(target_os = "android")]
    pub fn app_info(&self) -> Result<serde_json::Value, String> {
        self.0
            .run_mobile_plugin("appInfo", serde_json::json!({}))
            .map_err(|e| e.to_string())
    }
    #[cfg(target_os = "android")]
    pub fn open_web_url(&self, url: &str) -> Result<serde_json::Value, String> {
        self.0
            .run_mobile_plugin("openWebUrl", serde_json::json!({"url":url}))
            .map_err(|e| e.to_string())
    }
    pub fn save(&self, path: &str, filename: &str) -> Result<serde_json::Value, String> {
        self.0
            .run_mobile_plugin("save", serde_json::json!({"path":path,"filename":filename}))
            .map_err(|e| e.to_string())
    }
    pub fn open(&self, path: &str) -> Result<serde_json::Value, String> {
        self.0
            .run_mobile_plugin("open", serde_json::json!({"path":path}))
            .map_err(|e| e.to_string())
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("mobile-files")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle =
                    api.register_android_plugin("org.zimstudio.files", "MobileFilesPlugin")?;
                app.manage(MobileFiles(handle));
            }
            #[cfg(target_os = "ios")]
            {
                use tauri::Manager;
                let handle = api.register_ios_plugin(init_plugin_mobile_files)?;
                app.manage(MobileFiles(handle));
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}
