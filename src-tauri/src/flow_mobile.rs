//! Android adapters. Only Rust's explicit picker/clipboard commands call these methods.
use tauri::{
    plugin::{Builder, PluginHandle, TauriPlugin},
    Manager,
};

pub struct MobileFlow(pub PluginHandle<tauri::Wry>);

pub fn init() -> TauriPlugin<tauri::Wry> {
    Builder::new("argon-mobile")
        .setup(|app, api| {
            let handle = api.register_android_plugin("com.otium.downloader", "FlowFilesPlugin")?;
            app.manage(MobileFlow(handle));
            Ok(())
        })
        .build()
}

pub async fn call<T: serde::de::DeserializeOwned + Send + 'static>(
    app: tauri::AppHandle,
    method: &'static str,
    payload: serde_json::Value,
) -> Result<T, String> {
    tokio::task::spawn_blocking(move || {
        app.state::<MobileFlow>()
            .0
            .run_mobile_plugin(method, payload)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
