#[cfg(desktop)]
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

#[cfg(desktop)]
pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "tray_show", "إظهار ARGON / Show ARGON", true, None::<&str>)?;
    let pause_all = MenuItem::with_id(
        app,
        "tray_pause_all",
        "إيقاف كل التنزيلات / Pause All",
        true,
        None::<&str>,
    )?;
    let resume_all = MenuItem::with_id(
        app,
        "tray_resume_all",
        "استئناف كل التنزيلات / Resume All",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "tray_quit", "إنهاء ARGON / Quit", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(app, &[&show, &pause_all, &resume_all, &sep, &quit])?;

    let mut builder = TrayIconBuilder::with_id("argon-tray")
        .menu(&menu)
        .tooltip("ARGON — جاهز للتحميل والمشاركة")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "tray_show" => show_main_window(app),
            "tray_pause_all" => {
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Ok(dl) = crate::ensure_downloader(&handle).await {
                        let _ = dl.pause_all().await;
                    }
                });
            }
            "tray_resume_all" => {
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Ok(dl) = crate::ensure_downloader(&handle).await {
                        let _ = dl.resume_all().await;
                    }
                });
            }
            "tray_quit" => {
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(dl) = handle
                        .state::<crate::DownloaderRuntime>()
                        .service
                        .lock()
                        .await
                        .clone()
                    {
                        dl.shutdown().await;
                    }
                    if let Some(engine) = handle
                        .state::<crate::flow::FlowRuntime>()
                        .engine
                        .lock()
                        .await
                        .clone()
                    {
                        engine.stop().await;
                    }
                    handle.exit(0);
                });
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}

#[cfg(desktop)]
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn sync_windows_autostart(enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use winreg::{
            enums::{HKEY_CURRENT_USER, KEY_WRITE},
            RegKey,
        };
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu
            .create_subkey_with_flags(
                r"Software\Microsoft\Windows\CurrentVersion\Run",
                KEY_WRITE,
            )
            .map_err(|e| format!("Cannot update startup registry: {e}"))?;
        if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let cmd = format!("\"{}\" --minimized", exe.display());
            key.set_value("ARGON", &cmd)
                .map_err(|e| format!("Cannot set startup value: {e}"))?;
        } else {
            let _ = key.delete_value("ARGON");
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        Ok(())
    }
}
