use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};
#[cfg(desktop)]
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex;

#[derive(Default)]
pub struct UpdateRuntime {
    operation: Mutex<()>,
    cache: Mutex<Cache>,
}
#[derive(Default)]
struct Cache {
    #[cfg(desktop)]
    update: Option<Update>,
    phase: String,
    downloaded: u64,
    total: Option<u64>,
    ready: bool,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    mode: String,
    phase: String,
    version: Option<String>,
    notes: Option<String>,
    downloaded: u64,
    total: Option<u64>,
}
#[derive(Serialize, Deserialize)]
struct CachedPackage {
    version: String,
    signature: String,
    sha256: String,
}

fn package_mode() -> &'static str {
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        use winreg::{
            enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WOW64_64KEY},
            RegKey,
        };
        let installed = || -> Option<bool> {
            let key = RegKey::predef(HKEY_CURRENT_USER)
                .open_subkey_with_flags(
                    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\ARGON",
                    KEY_READ | KEY_WOW64_64KEY,
                )
                .ok()?;
            let location: String = key.get_value("InstallLocation").ok()?;
            let registered = std::fs::canonicalize(location.trim_matches('"')).ok()?;
            let executable = std::env::current_exe().ok()?;
            let running = std::fs::canonicalize(executable.parent()?).ok()?;
            Some(registered == running && running.join("uninstall.exe").is_file())
        };
        return if installed() == Some(true) {
            "installed"
        } else {
            "portable"
        };
    }
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        "unsupported"
    }
}

fn cache_directory(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("signed-updates"))
}
fn valid_asset(version: &str, url: &str, signature: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    let version_ok = version.split('.').count() == 3
        && version
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    version_ok
        && !signature.trim().is_empty()
        && parsed.scheme() == "https"
        && parsed.host_str() == Some("github.com")
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.port().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && parsed.path()
            == format!("/XRTM8/ARGON/releases/download/v{version}/ARGON_{version}_x64-setup.exe")
}
fn cache_matches(record: &CachedPackage, version: &str, signature: &str, bytes: &[u8]) -> bool {
    record.version == version
        && record.signature == signature
        && record.sha256 == format!("{:x}", Sha256::digest(bytes))
}
async fn cached_bytes(app: &AppHandle, version: &str, signature: &str) -> Option<Vec<u8>> {
    let root = cache_directory(app).ok()?;
    if tokio::fs::metadata(root.join("package.bin"))
        .await
        .ok()?
        .len()
        > 256 * 1024 * 1024
    {
        return None;
    }
    let record: CachedPackage =
        serde_json::from_slice(&tokio::fs::read(root.join("package.json")).await.ok()?).ok()?;
    let bytes = tokio::fs::read(root.join("package.bin")).await.ok()?;
    cache_matches(&record, version, signature, &bytes).then_some(bytes)
}
async fn save_package(
    app: &AppHandle,
    version: &str,
    signature: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let root = cache_directory(app)?;
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| e.to_string())?;
    let record = CachedPackage {
        version: version.into(),
        signature: signature.into(),
        sha256: format!("{:x}", Sha256::digest(bytes)),
    };
    for (name, content) in [
        ("package.bin", bytes.to_vec()),
        (
            "package.json",
            serde_json::to_vec(&record).map_err(|e| e.to_string())?,
        ),
    ] {
        let temporary = root.join(format!("{name}.tmp"));
        tokio::fs::write(&temporary, content)
            .await
            .map_err(|e| e.to_string())?;
        let destination = root.join(name);
        if destination.exists() {
            tokio::fs::remove_file(&destination)
                .await
                .map_err(|e| e.to_string())?;
        }
        tokio::fs::rename(temporary, destination)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn status(cache: &Cache) -> UpdateStatus {
    #[cfg(desktop)]
    let (version, notes) = (
        cache.update.as_ref().map(|u| u.version.clone()),
        cache.update.as_ref().and_then(|u| u.body.clone()),
    );
    #[cfg(not(desktop))]
    let (version, notes) = (None, None);
    UpdateStatus {
        mode: package_mode().into(),
        phase: if cache.phase.is_empty() {
            "idle".into()
        } else {
            cache.phase.clone()
        },
        version,
        notes,
        downloaded: cache.downloaded,
        total: cache.total,
    }
}

#[tauri::command]
pub async fn update_status(state: tauri::State<'_, UpdateRuntime>) -> Result<UpdateStatus, String> {
    let cache = state.cache.lock().await;
    Ok(status(&cache))
}

#[tauri::command]
pub async fn update_check(
    app: AppHandle,
    state: tauri::State<'_, UpdateRuntime>,
) -> Result<UpdateStatus, String> {
    if package_mode() != "installed" {
        return Err("unsupported_package".into());
    }
    let _operation = state.operation.try_lock().map_err(|_| "update_busy")?;
    #[cfg(desktop)]
    {
        {
            let cache = state.cache.lock().await;
            if cache.ready {
                return Ok(status(&cache));
            }
        }
        let updater = app
            .updater_builder()
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|e| e.to_string())?;
        let update = tokio::time::timeout(Duration::from_secs(20), updater.check())
            .await
            .map_err(|_| "update_check_timeout")?
            .map_err(|e| e.to_string())?;
        if let Some(ref value) = update {
            if !valid_asset(
                &value.version,
                value.download_url.as_str(),
                &value.signature,
            ) {
                return Err("incompatible_update_package".into());
            }
        }
        let bytes = if let Some(ref value) = update {
            cached_bytes(&app, &value.version, &value.signature)
                .await
                .filter(|bytes| value.verify(bytes).is_ok())
        } else {
            None
        };
        let mut cache = state.cache.lock().await;
        cache.ready = bytes.is_some();
        cache.downloaded = bytes.as_ref().map_or(0, |b| b.len() as u64);
        cache.total = bytes.as_ref().map(|b| b.len() as u64);
        cache.phase = if cache.ready {
            "ready"
        } else if update.is_some() {
            "available"
        } else {
            "current"
        }
        .into();
        cache.update = update;
        return Ok(status(&cache));
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Err("unsupported_package".into())
    }
}

#[tauri::command]
pub async fn update_download(
    app: AppHandle,
    state: tauri::State<'_, UpdateRuntime>,
) -> Result<UpdateStatus, String> {
    if package_mode() != "installed" {
        return Err("unsupported_package".into());
    }
    let _operation = state.operation.try_lock().map_err(|_| "update_busy")?;
    #[cfg(desktop)]
    {
        let update = {
            let mut cache = state.cache.lock().await;
            if cache.ready {
                return Ok(status(&cache));
            }
            let update = cache.update.clone().ok_or("check_for_updates_first")?;
            cache.phase = "downloading".into();
            update
        };
        let mut downloaded = 0u64;
        let result = update
            .download(
                |chunk, total| {
                    downloaded += chunk as u64;
                    let _ = app.emit(
                        "update-progress",
                        serde_json::json!({ "downloaded": downloaded, "total": total }),
                    );
                },
                || {},
            )
            .await;
        let bytes = match result {
            Ok(bytes) => bytes,
            Err(error) => {
                state.cache.lock().await.phase = "available".into();
                return Err(error.to_string());
            }
        };
        if let Err(error) = save_package(&app, &update.version, &update.signature, &bytes).await {
            state.cache.lock().await.phase = "available".into();
            return Err(error);
        }
        let mut cache = state.cache.lock().await;
        cache.ready = true;
        cache.phase = "ready".into();
        cache.downloaded = bytes.len() as u64;
        cache.total = Some(bytes.len() as u64);
        return Ok(status(&cache));
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Err("unsupported_package".into())
    }
}

#[tauri::command]
pub async fn update_install(
    app: AppHandle,
    state: tauri::State<'_, UpdateRuntime>,
    tasks: tauri::State<'_, Arc<crate::TaskManager>>,
) -> Result<(), String> {
    if package_mode() != "installed" {
        return Err("unsupported_package".into());
    }
    let _operation = state.operation.try_lock().map_err(|_| "update_busy")?;
    #[cfg(desktop)]
    {
        let update = {
            let cache = state.cache.lock().await;
            if !cache.ready {
                return Err("download_update_first".into());
            }
            cache.update.clone().ok_or("check_for_updates_first")?
        };
        let Some(bytes) = cached_bytes(&app, &update.version, &update.signature).await else {
            let mut cache = state.cache.lock().await;
            cache.ready = false;
            cache.phase = "available".into();
            return Err("cached_update_changed".into());
        };
        if let Err(error) = update.verify(&bytes) {
            let mut cache = state.cache.lock().await;
            cache.ready = false;
            cache.phase = "available".into();
            return Err(error.to_string());
        }
        if tasks.updating.swap(true, Ordering::SeqCst) {
            return Err("update_busy".into());
        }
        let result = async {
            if !tasks.cancellations.read().await.is_empty() {
                return Err("transfers_active".to_string());
            }
            let engine = app
                .state::<crate::flow::FlowRuntime>()
                .engine
                .lock()
                .await
                .clone();
            let mut was_enabled = false;
            if let Some(ref engine) = engine {
                let snapshot = engine.snapshot().await;
                if !snapshot.incoming.is_empty()
                    || !snapshot.pairings.is_empty()
                    || snapshot.transfers.iter().any(|t| {
                        [
                            "queued",
                            "hashing",
                            "waiting",
                            "sending",
                            "receiving",
                            "verifying",
                        ]
                        .contains(&t.status.as_str())
                    })
                {
                    return Err("transfers_active".to_string());
                }
                was_enabled = snapshot.enabled;
                engine.stop().await;
            }
            // The official updater verifies the signature again immediately before launching NSIS.
            let result = update.install(&bytes).map_err(|e| e.to_string());
            if result.is_err() {
                let mut cache = state.cache.lock().await;
                cache.ready = false;
                cache.phase = "available".into();
                drop(cache);
                if was_enabled {
                    if let Some(engine) = engine {
                        let _ = engine.start().await;
                    }
                }
            }
            result
        }
        .await;
        tasks.updating.store(false, Ordering::SeqCst);
        result
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, tasks);
        Err("unsupported_package".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_only_the_versioned_argon_nsis_package() {
        let url =
            "https://github.com/XRTM8/ARGON/releases/download/v2.3.0/ARGON_2.3.0_x64-setup.exe";
        assert!(valid_asset("2.3.0", url, "signed"));
        for invalid in [
            url.replace("https:", "http:"),
            url.replace("XRTM8/ARGON", "other/ARGON"),
            url.replace("v2.3.0/", "v2.2.0/"),
            url.replace("_x64-setup.exe", "-portable.zip"),
            format!("{url}?override=1"),
        ] {
            assert!(!valid_asset("2.3.0", &invalid, "signed"));
        }
        assert!(!valid_asset("2.3.0", url, ""));
        assert!(!valid_asset("2.3.0-beta", url, "signed"));
    }
    #[test]
    fn refuses_a_stale_or_modified_cached_update() {
        let record = CachedPackage {
            version: "2.3.0".into(),
            signature: "signature".into(),
            sha256: format!("{:x}", Sha256::digest(b"package")),
        };
        assert!(cache_matches(&record, "2.3.0", "signature", b"package"));
        assert!(!cache_matches(
            &record,
            "2.3.0",
            "different-signature",
            b"package"
        ));
        assert!(!cache_matches(&record, "2.4.0", "signature", b"package"));
        assert!(!cache_matches(&record, "2.3.0", "signature", b"tampered"));
    }
    #[test]
    #[ignore = "release pipeline provides the real signed installer"]
    fn release_signature_matches_embedded_key() {
        use base64::Engine;
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let public = base64::engine::general_purpose::STANDARD
            .decode(config["plugins"]["updater"]["pubkey"].as_str().unwrap())
            .unwrap();
        let installer = std::env::var("ARGON_SIGNED_INSTALLER").expect("installer path");
        let encoded = std::fs::read_to_string(format!("{installer}.sig")).unwrap();
        let signature = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .unwrap();
        let key =
            minisign_verify::PublicKey::decode(std::str::from_utf8(&public).unwrap()).unwrap();
        let signature =
            minisign_verify::Signature::decode(std::str::from_utf8(&signature).unwrap()).unwrap();
        key.verify(&std::fs::read(installer).unwrap(), &signature, true)
            .expect("release signature must match the public key embedded in ARGON");
        assert_eq!(
            signature
                .trusted_comment()
                .split('\t')
                .find_map(|field| field.strip_prefix("version:")),
            config["version"].as_str(),
            "the signature must bind the installer to the announced app version"
        );
    }
}
