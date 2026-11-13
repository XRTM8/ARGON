use argon_flow_core::{Config, FlowService, SelectedFile, Snapshot};
use std::{path::PathBuf, sync::Arc};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Mutex;

#[derive(Default)]
pub struct FlowRuntime {
    pub engine: Mutex<Option<Arc<FlowService>>>,
}
async fn service(app: &AppHandle, state: &FlowRuntime) -> Result<Arc<FlowService>, String> {
    if app
        .state::<Arc<crate::TaskManager>>()
        .updating
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err("update_installing".into());
    }
    let mut engine = state.engine.lock().await;
    if let Some(engine) = engine.as_ref() {
        return Ok(engine.clone());
    }
    let storage = app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("flow-v2");
    #[cfg(not(target_os = "android"))]
    let receive = crate::get_resolved_download_dir(app).join("ARGON Received");
    // Internal app storage supports atomic hard-link commits, unlike Android's emulated SD storage.
    #[cfg(target_os = "android")]
    let receive = app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("received");
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "ARGON device".into());
    let app = app.clone();
    let created = FlowService::new(Config::desktop(storage, receive, name), move |event| {
        let _ = app.emit("flow-event", event);
    })
    .await?;
    *engine = Some(created.clone());
    Ok(created)
}

#[tauri::command]
pub async fn flow_snapshot(
    app: AppHandle,
    state: tauri::State<'_, FlowRuntime>,
) -> Result<Snapshot, String> {
    Ok(service(&app, &state).await?.snapshot().await)
}
#[tauri::command]
pub async fn flow_start(
    app: AppHandle,
    state: tauri::State<'_, FlowRuntime>,
) -> Result<Snapshot, String> {
    let engine = service(&app, &state).await?;
    engine.start().await?;
    Ok(engine.snapshot().await)
}
#[tauri::command]
pub async fn flow_stop(
    app: AppHandle,
    state: tauri::State<'_, FlowRuntime>,
) -> Result<Snapshot, String> {
    let engine = service(&app, &state).await?;
    engine.stop().await;
    Ok(engine.snapshot().await)
}
#[tauri::command]
pub async fn flow_connect(
    app: AppHandle,
    state: tauri::State<'_, FlowRuntime>,
    address: String,
) -> Result<(), String> {
    service(&app, &state).await?.connect(&address).await
}
#[tauri::command]
pub async fn flow_pair_peer(
    app: AppHandle,
    state: tauri::State<'_, FlowRuntime>,
    peer_id: String,
) -> Result<(), String> {
    service(&app, &state).await?.pair_peer(&peer_id).await
}
#[tauri::command]
pub async fn flow_confirm_pairing(
    app: AppHandle,
    state: tauri::State<'_, FlowRuntime>,
    request_id: String,
    accepted: bool,
) -> Result<(), String> {
    service(&app, &state)
        .await?
        .confirm_pairing(&request_id, accepted)
        .await
}
#[tauri::command]
pub async fn flow_respond_incoming(
    app: AppHandle,
    state: tauri::State<'_, FlowRuntime>,
    request_id: String,
    accepted: bool,
) -> Result<(), String> {
    service(&app, &state)
        .await?
        .respond_incoming(&request_id, accepted)
        .await
}
#[tauri::command]
pub async fn flow_revoke_peer(
    app: AppHandle,
    state: tauri::State<'_, FlowRuntime>,
    peer_id: String,
) -> Result<(), String> {
    service(&app, &state).await?.revoke(&peer_id).await
}
#[tauri::command]
pub async fn flow_cancel_transfer(
    app: AppHandle,
    state: tauri::State<'_, FlowRuntime>,
    transfer_id: String,
) -> Result<(), String> {
    service(&app, &state).await?.cancel(&transfer_id).await
}
#[tauri::command]
pub async fn flow_send_files(
    app: AppHandle,
    state: tauri::State<'_, FlowRuntime>,
    peer_id: String,
    file_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    service(&app, &state)
        .await?
        .send_files(&peer_id, file_ids)
        .await
}
#[tauri::command]
pub async fn flow_send_text(
    app: AppHandle,
    state: tauri::State<'_, FlowRuntime>,
    peer_id: String,
    text: String,
) -> Result<String, String> {
    service(&app, &state).await?.send_text(&peer_id, text).await
}

#[tauri::command]
pub async fn flow_choose_files(
    app: AppHandle,
    state: tauri::State<'_, FlowRuntime>,
    hint: Option<String>,
) -> Result<Vec<SelectedFile>, String> {
    #[cfg(not(target_os = "android"))]
    {
        let engine = service(&app, &state).await?;
        let mut dialog = rfd::AsyncFileDialog::new().set_title("ARGON — Choose files to share");
        if let Some(hint) = hint {
            let path = PathBuf::from(hint);
            if path.is_absolute() {
                if let Some(parent) = path.parent() {
                    dialog = dialog.set_directory(parent);
                }
                if let Some(name) = path.file_name() {
                    dialog = dialog.set_file_name(name.to_string_lossy());
                }
            }
        }
        let Some(handles) = dialog.pick_files().await else {
            return Ok(vec![]);
        };
        engine
            .select_files(handles.iter().map(|h| h.path().to_path_buf()).collect())
            .await
    }
    #[cfg(target_os = "android")]
    {
        let _ = hint;
        #[derive(serde::Deserialize)]
        struct Picked {
            paths: Vec<PathBuf>,
        }
        let picked: Picked =
            crate::flow_mobile::call(app.clone(), "chooseFiles", serde_json::json!({})).await?;
        service(&app, &state)
            .await?
            .select_files(picked.paths)
            .await
    }
}

#[tauri::command]
pub async fn flow_choose_receive_directory(
    app: AppHandle,
    state: tauri::State<'_, FlowRuntime>,
) -> Result<Snapshot, String> {
    #[cfg(not(target_os = "android"))]
    {
        let engine = service(&app, &state).await?;
        if let Some(folder) = rfd::AsyncFileDialog::new()
            .set_title("ARGON — Receive directory")
            .pick_folder()
            .await
        {
            engine
                .set_receive_directory(folder.path().to_path_buf())
                .await?;
        }
        Ok(engine.snapshot().await)
    }
    #[cfg(target_os = "android")]
    {
        let _ = (app, state);
        Err("تغيير مجلد الاستقبال غير متاح على Android حاليًا".into())
    }
}

#[tauri::command]
pub async fn flow_clean_text(text: String) -> Result<(String, bool), String> {
    if text.len() > 16 * 1024 {
        return Err("Text exceeds 16 KB".into());
    }
    Ok(argon_flow_core::clean_text(&text))
}

#[tauri::command]
pub async fn flow_open_received(
    app: AppHandle,
    state: tauri::State<'_, FlowRuntime>,
    transfer_id: String,
) -> Result<(), String> {
    let engine = service(&app, &state).await?;
    let transfer = engine
        .snapshot()
        .await
        .transfers
        .into_iter()
        .find(|t| t.id == transfer_id && t.status == "completed" && t.direction == "receive")
        .ok_or("Received file is no longer available")?;
    let path = transfer
        .path
        .ok_or("This transfer does not contain a file")?;
    #[cfg(target_os = "android")]
    {
        crate::flow_mobile::call::<serde_json::Value>(
            app,
            "openFile",
            serde_json::json!({"path":path}),
        )
        .await?;
        Ok(())
    }
    #[cfg(not(target_os = "android"))]
    {
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .reveal_item_in_dir(path)
            .map_err(|e| e.to_string())
    }
}
#[tauri::command]
pub async fn flow_read_clipboard(_app: AppHandle) -> Result<String, String> {
    #[cfg(not(target_os = "android"))]
    {
        tokio::task::spawn_blocking(|| {
            arboard::Clipboard::new()
                .map_err(|e| e.to_string())?
                .get_text()
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(target_os = "android")]
    {
        #[derive(serde::Deserialize)]
        struct Clipboard {
            text: String,
        }
        let result: Clipboard =
            crate::flow_mobile::call(_app, "readClipboard", serde_json::json!({})).await?;
        Ok(result.text)
    }
}
#[tauri::command]
pub async fn flow_write_clipboard(_app: AppHandle, text: String) -> Result<(), String> {
    #[cfg(not(target_os = "android"))]
    {
        tokio::task::spawn_blocking(move || {
            arboard::Clipboard::new()
                .map_err(|e| e.to_string())?
                .set_text(text)
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(target_os = "android")]
    {
        crate::flow_mobile::call::<serde_json::Value>(
            _app,
            "writeClipboard",
            serde_json::json!({"text": text}),
        )
        .await?;
        Ok(())
    }
}
#[tauri::command]
pub async fn flow_copy_received(
    app: AppHandle,
    state: tauri::State<'_, FlowRuntime>,
    item_id: String,
) -> Result<(), String> {
    let engine = service(&app, &state).await?;
    let item = engine
        .snapshot()
        .await
        .clipboard
        .into_iter()
        .find(|item| item.id == item_id)
        .ok_or("Clipboard item is no longer available")?;
    #[cfg(not(target_os = "android"))]
    {
        tokio::task::spawn_blocking(move || {
            arboard::Clipboard::new()
                .map_err(|e| e.to_string())?
                .set_text(item.text)
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(target_os = "android")]
    {
        crate::flow_mobile::call::<serde_json::Value>(
            app,
            "writeClipboard",
            serde_json::json!({"text":item.text}),
        )
        .await?;
        Ok(())
    }
}
