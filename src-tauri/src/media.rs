use serde::Serialize;
use std::{collections::HashMap, time::Duration};
use tauri::{AppHandle, Manager};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedMedia {
    download_url: String,
    suggested_filename: String,
    headers: HashMap<String, String>,
}

#[tauri::command]
pub async fn resolve_media(
    app: AppHandle,
    url: String,
    audio_only: bool,
) -> Result<ResolvedMedia, String> {
    let parsed = reqwest::Url::parse(&url).map_err(|_| "Invalid media URL")?;
    if !["https", "http"].contains(&parsed.scheme()) {
        return Err("Unsupported media protocol".into());
    }
    #[cfg(target_os = "android")]
    {
        let _ = (app, audio_only);
        Err("استخراج روابط المنصات غير متاح على Android حاليًا؛ استخدم رابط الملف المباشر".into())
    }
    #[cfg(not(target_os = "android"))]
    {
        let tools = app
            .path()
            .resource_dir()
            .map_err(|e| e.to_string())?
            .join("tools");
        let executable = tools.join(if cfg!(windows) {
            "yt-dlp.exe"
        } else {
            "yt-dlp"
        });
        let fallback = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tools")
            .join(if cfg!(windows) {
                "yt-dlp.exe"
            } else {
                "yt-dlp"
            });
        let executable = if executable.exists() {
            executable
        } else if cfg!(debug_assertions) && fallback.exists() {
            fallback
        } else {
            "yt-dlp".into()
        };
        let mut command = tokio::process::Command::new(executable);
        command.kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let node = tools.join(if cfg!(windows) { "node.exe" } else { "node" });
        let node = if node.exists() {
            format!("node:{}", node.display())
        } else {
            "node".into()
        };
        command.args(["--ignore-config", "--no-playlist", "--no-warnings", "--skip-download", "--dump-single-json",
            "--socket-timeout", "20", "--retries", "2", "--js-runtimes", &node, "--format",
            if audio_only { "bestaudio[protocol=https]/bestaudio[protocol=http]/best[protocol=https]/best[protocol=http]" }
            else { "best[protocol=https]/best[protocol=http]" }, "--", &url]);
        let output = tokio::time::timeout(Duration::from_secs(90), command.output())
            .await
            .map_err(|_| "انتهت مهلة استخراج رابط الوسائط؛ حاول مجددًا")?
            .map_err(|e| {
                format!("محرك الوسائط غير متاح؛ أعد تثبيت ARGON أو شغّل npm run prepare:tools. {e}")
            })?;
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "تعذر استخراج ملف مباشر: {}",
                error.chars().take(1000).collect::<String>()
            ));
        }
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).map_err(|_| "Invalid media engine response")?;
        let stream = value["url"]
            .as_str()
            .ok_or("لم تتوفر صيغة تنزيل مباشر لهذا الرابط")?;
        let stream_url = reqwest::Url::parse(stream).map_err(|_| "Invalid extracted URL")?;
        if !["http", "https"].contains(&stream_url.scheme()) {
            return Err("Unsupported stream protocol".into());
        }
        let ext = value["ext"].as_str().unwrap_or("mp4");
        let title = crate::sanitize_filename(value["title"].as_str().unwrap_or("media"));
        let mut headers = HashMap::new();
        if let Some(map) = value["http_headers"].as_object() {
            for (key, value) in map {
                if let Some(value) = value.as_str() {
                    headers.insert(key.clone(), value.to_owned());
                }
            }
        }
        Ok(ResolvedMedia {
            download_url: stream.to_owned(),
            suggested_filename: format!("{}.{}", title.chars().take(160).collect::<String>(), ext),
            headers,
        })
    }
}
