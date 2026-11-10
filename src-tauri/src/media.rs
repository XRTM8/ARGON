use crate::{SegmentProgressEvent, TaskManager};
use serde::Serialize;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedMedia {
    download_url: String,
    suggested_filename: String,
    headers: HashMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaFormatInfo {
    pub id: String,
    pub ext: String,
    pub resolution: String,
    pub fps: Option<u32>,
    pub filesize: Option<u64>,
    pub vcodec: String,
    pub acodec: String,
    pub has_video: bool,
    pub has_audio: bool,
    pub label: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMetadataInfo {
    pub title: String,
    pub uploader: Option<String>,
    pub thumbnail: Option<String>,
    pub duration: Option<f64>,
    pub has_ffmpeg: bool,
    pub formats: Vec<MediaFormatInfo>,
}

#[cfg(not(target_os = "android"))]
fn resolve_tool_paths(app: &AppHandle) -> Result<(PathBuf, String, Option<PathBuf>), String> {
    let tools = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join("tools");
    let manifest_tools = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools");
    let find_bin = |win_name: &str, unix_name: &str| -> Option<PathBuf> {
        let name = if cfg!(windows) { win_name } else { unix_name };
        let res = tools.join(name);
        if res.exists() {
            return Some(res);
        }
        let dev = manifest_tools.join(name);
        if cfg!(debug_assertions) && dev.exists() {
            return Some(dev);
        }
        None
    };

    let ytdlp = find_bin("yt-dlp.exe", "yt-dlp").unwrap_or_else(|| PathBuf::from("yt-dlp"));
    let node = find_bin("node.exe", "node")
        .map(|p| format!("node:{}", p.display()))
        .unwrap_or_else(|| "node".to_string());
    let ffmpeg = find_bin("ffmpeg.exe", "ffmpeg");
    Ok((ytdlp, node, ffmpeg))
}

#[cfg(not(target_os = "android"))]
async fn run_ytdlp_json(
    app: &AppHandle,
    url: &str,
    format_arg: Option<&str>,
) -> Result<serde_json::Value, String> {
    let (executable, node, _) = resolve_tool_paths(app)?;
    let mut command = tokio::process::Command::new(executable);
    command.kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command.args([
        "--ignore-config",
        "--no-playlist",
        "--no-warnings",
        "--skip-download",
        "--dump-single-json",
        "--socket-timeout",
        "20",
        "--retries",
        "2",
        "--js-runtimes",
        &node,
    ]);
    if let Some(fmt) = format_arg {
        command.args(["--format", fmt]);
    }
    command.args(["--", url]);

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
    serde_json::from_slice(&output.stdout).map_err(|_| "Invalid media engine response".to_string())
}

#[tauri::command]
pub async fn list_media_formats(app: AppHandle, url: String) -> Result<MediaMetadataInfo, String> {
    let parsed = reqwest::Url::parse(&url).map_err(|_| "Invalid media URL")?;
    if !["https", "http"].contains(&parsed.scheme()) {
        return Err("Unsupported media protocol".into());
    }
    #[cfg(target_os = "android")]
    {
        let _ = app;
        Err("استخراج روابط المنصات غير متاح على Android حاليًا؛ استخدم رابط الملف المباشر".into())
    }
    #[cfg(not(target_os = "android"))]
    {
        let (_, _, ffmpeg) = resolve_tool_paths(&app)?;
        let has_ffmpeg = ffmpeg.is_some();
        let value = run_ytdlp_json(&app, &url, None).await?;
        let title = value["title"].as_str().unwrap_or("media").to_string();
        let uploader = value["uploader"].as_str().map(str::to_string);
        let thumbnail = value["thumbnail"].as_str().map(str::to_string);
        let duration = value["duration"].as_f64();

        let mut formats = Vec::new();
        if let Some(list) = value["formats"].as_array() {
            for f in list {
                let Some(id) = f["format_id"].as_str() else {
                    continue;
                };
                let protocol = f["protocol"].as_str().unwrap_or("");
                let vcodec = f["vcodec"].as_str().unwrap_or("none").to_string();
                let acodec = f["acodec"].as_str().unwrap_or("none").to_string();
                let has_video = vcodec != "none";
                let has_audio = acodec != "none";
                if !has_video && !has_audio {
                    continue;
                }
                let is_direct = protocol == "https" || protocol == "http";
                if !has_ffmpeg && (!is_direct || (has_video && !has_audio)) {
                    continue;
                }
                let ext = f["ext"].as_str().unwrap_or("mp4").to_string();
                let height = f["height"].as_u64();
                let resolution = height
                    .map(|h| format!("{h}p"))
                    .or_else(|| f["resolution"].as_str().map(str::to_string))
                    .unwrap_or_else(|| if has_video { "Video".into() } else { "Audio".into() });
                let fps = f["fps"].as_f64().map(|v| v.round() as u32);
                let filesize = f["filesize"].as_u64().or_else(|| f["filesize_approx"].as_u64());
                let label = if has_video && has_audio {
                    format!("{resolution} · {ext}")
                } else if has_video {
                    format!(
                        "{resolution}{} · {ext} (+audio)",
                        fps.map(|v| format!(" {v}fps")).unwrap_or_default()
                    )
                } else {
                    format!("Audio · {ext}")
                };
                formats.push(MediaFormatInfo {
                    id: id.to_string(),
                    ext,
                    resolution,
                    fps,
                    filesize,
                    vcodec,
                    acodec,
                    has_video,
                    has_audio,
                    label,
                });
            }
        }
        Ok(MediaMetadataInfo {
            title,
            uploader,
            thumbnail,
            duration,
            has_ffmpeg,
            formats,
        })
    }
}

#[tauri::command]
pub async fn resolve_media(
    app: AppHandle,
    url: String,
    audio_only: bool,
    format_id: Option<String>,
) -> Result<ResolvedMedia, String> {
    let parsed = reqwest::Url::parse(&url).map_err(|_| "Invalid media URL")?;
    if !["https", "http"].contains(&parsed.scheme()) {
        return Err("Unsupported media protocol".into());
    }
    #[cfg(target_os = "android")]
    {
        let _ = (app, audio_only, format_id);
        Err("استخراج روابط المنصات غير متاح على Android حاليًا؛ استخدم رابط الملف المباشر".into())
    }
    #[cfg(not(target_os = "android"))]
    {
        let fmt = format_id.unwrap_or_else(|| {
            if audio_only {
                "bestaudio[protocol=https]/bestaudio[protocol=http]/best[protocol=https]/best[protocol=http]".to_string()
            } else {
                "best[protocol=https]/best[protocol=http]".to_string()
            }
        });
        let value = run_ytdlp_json(&app, &url, Some(&fmt)).await?;
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

#[tauri::command]
pub async fn download_media_with_ytdlp(
    app: AppHandle,
    state: tauri::State<'_, Arc<TaskManager>>,
    task_id: String,
    url: String,
    destination_path: String,
    format_spec: String,
) -> Result<String, String> {
    let parsed = reqwest::Url::parse(&url).map_err(|_| "Invalid media URL")?;
    if !["https", "http"].contains(&parsed.scheme()) {
        return Err("Unsupported media protocol".into());
    }
    #[cfg(target_os = "android")]
    {
        let _ = (app, state, task_id, destination_path, format_spec);
        Err("تنزيل HLS/DASH عبر yt-dlp متاح على سطح المكتب فقط حاليًا".into())
    }
    #[cfg(not(target_os = "android"))]
    {
        let token = state.begin_transfer(&task_id).await?;
        let mut cancel_rx = state.subscribe_cancel(&task_id).await;
        let (executable, node, ffmpeg) = resolve_tool_paths(&app)?;
        let target = crate::resolve_destination_path(&app, &destination_path);
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Cannot create download folder: {e}"))?;
        }
        let mut command = tokio::process::Command::new(executable);
        command.kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        command.args([
            "--ignore-config",
            "--no-playlist",
            "--no-warnings",
            "--newline",
            "--progress",
            "--concurrent-fragments",
            "4",
            "--js-runtimes",
            &node,
            "--format",
            &format_spec,
            "-o",
            &target.to_string_lossy(),
        ]);
        if let Some(ff) = ffmpeg {
            if let Some(dir) = ff.parent() {
                command.args(["--ffmpeg-location", &dir.to_string_lossy()]);
            }
        }
        command.args(["--", &url]);
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());

        let mut child = command
            .spawn()
            .map_err(|e| format!("تعذر تشغيل محرك الوسائط: {e}"))?;
        let stdout = child.stdout.take();
        if let Some(out) = stdout {
            use tokio::io::AsyncBufReadExt;
            let mut lines = tokio::io::BufReader::new(out).lines();
            loop {
                if token.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = child.kill().await;
                    return Err("CANCELLED".into());
                }
                let next_line = tokio::select! {
                    line = lines.next_line() => line,
                    _ = crate::wait_cancelled(&mut cancel_rx, &token) => {
                        let _ = child.kill().await;
                        return Err("CANCELLED".into());
                    }
                };
                match next_line {
                    Ok(Some(line)) => {
                        if let Some((dl, total)) = parse_ytdlp_progress(&line) {
                            let _ = app.emit(
                                "download-progress",
                                SegmentProgressEvent {
                                    task_id: task_id.clone(),
                                    segment_index: 0,
                                    delta_bytes: 0,
                                    downloaded_bytes: dl,
                                    total_bytes: total,
                                    is_finished: false,
                                },
                            );
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        }
        let status = child
            .wait()
            .await
            .map_err(|e| format!("Media download error: {e}"))?;
        if !status.success() {
            return Err("فشل تنزيل الوسائط أو دمجها؛ تأكد من توفر ffmpeg وصلاحية الرابط.".into());
        }
        let final_size = tokio::fs::metadata(&target)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let _ = app.emit(
            "download-progress",
            SegmentProgressEvent {
                task_id,
                segment_index: 0,
                delta_bytes: 0,
                downloaded_bytes: final_size,
                total_bytes: final_size,
                is_finished: true,
            },
        );
        Ok(target.to_string_lossy().into_owned())
    }
}

#[cfg(not(target_os = "android"))]
fn parse_ytdlp_progress(line: &str) -> Option<(u64, u64)> {
    if !line.contains("[download]") || !line.contains('%') {
        return None;
    }
    let pct_pos = line.find('%')?;
    let pct_str = line[..pct_pos].split_whitespace().last()?;
    let pct: f64 = pct_str.parse().ok()?;
    let after_of = line.split(" of ").nth(1)?;
    let size_token = after_of
        .trim()
        .trim_start_matches('~')
        .split_whitespace()
        .next()?;
    let total = parse_human_size(size_token)?;
    let downloaded = ((pct / 100.0) * total as f64).round() as u64;
    Some((downloaded.min(total), total))
}

#[cfg(not(target_os = "android"))]
fn parse_human_size(token: &str) -> Option<u64> {
    let units = [
        ("GiB", 1024.0 * 1024.0 * 1024.0),
        ("MiB", 1024.0 * 1024.0),
        ("KiB", 1024.0),
        ("GB", 1000.0 * 1000.0 * 1000.0),
        ("MB", 1000.0 * 1000.0),
        ("KB", 1000.0),
        ("B", 1.0),
    ];
    for (suffix, mult) in units {
        if let Some(num) = token.strip_suffix(suffix) {
            if let Ok(val) = num.parse::<f64>() {
                return Some((val * mult).round() as u64);
            }
        }
    }
    None
}
