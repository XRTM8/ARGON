use argon_downloader_core::{social, BoxFuture, MediaExtractor, ResolvedMediaSpec};
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{io::AsyncWriteExt, sync::watch};

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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaToolsStatus {
    pub has_ytdlp: bool,
    pub has_node: bool,
    pub has_ffmpeg: bool,
    pub tools_dir: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolProgressEvent {
    tool: String,
    phase: String,
    downloaded: u64,
    total: Option<u64>,
}

#[cfg(not(target_os = "android"))]
fn user_tools_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("tools"))
}

#[cfg(not(target_os = "android"))]
pub fn resolve_tool_paths(app: &AppHandle) -> Result<(PathBuf, String, Option<PathBuf>), String> {
    let user_tools = user_tools_dir(app).ok();
    let res_tools = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join("tools");
    let manifest_tools = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools");
    let find_bin = |win_name: &str, unix_name: &str| -> Option<PathBuf> {
        let name = if cfg!(windows) { win_name } else { unix_name };
        if let Some(ref ut) = user_tools {
            let p = ut.join(name);
            if p.exists() {
                return Some(p);
            }
        }
        let res = res_tools.join(name);
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
pub fn has_ffmpeg_installed(app: &AppHandle) -> bool {
    resolve_tool_paths(app).is_ok_and(|(_, _, ff)| ff.is_some())
}

#[cfg(target_os = "android")]
pub fn has_ffmpeg_installed(_app: &AppHandle) -> bool {
    false
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
        // Auto-heal: Try updating yt-dlp once in background and retry extraction
        if update_ytdlp_tool(app.clone()).await.is_ok() {
            if let Ok((executable_retry, node_retry, _)) = resolve_tool_paths(app) {
                let mut retry_cmd = tokio::process::Command::new(executable_retry);
                retry_cmd.kill_on_drop(true);
                #[cfg(windows)]
                retry_cmd.creation_flags(0x08000000);
                retry_cmd.args([
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
                    &node_retry,
                ]);
                if let Some(fmt) = format_arg {
                    retry_cmd.args(["--format", fmt]);
                }
                retry_cmd.args(["--", url]);
                if let Ok(Ok(retry_output)) = tokio::time::timeout(Duration::from_secs(90), retry_cmd.output()).await {
                    if retry_output.status.success() {
                        return serde_json::from_slice(&retry_output.stdout)
                            .map_err(|_| "Invalid media engine response".to_string());
                    }
                }
            }
        }
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "تعذر استخراج ملف مباشر: {}",
            error.chars().take(1000).collect::<String>()
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|_| "Invalid media engine response".to_string())
}

pub struct TauriMediaExtractor {
    pub app: AppHandle,
}

impl MediaExtractor for TauriMediaExtractor {
    fn supports_hd_mux(&self) -> bool {
        has_ffmpeg_installed(&self.app)
    }

    fn resolve_media(
        &self,
        url: String,
        audio_only: bool,
        format_id: Option<String>,
    ) -> BoxFuture<'static, Result<ResolvedMediaSpec, String>> {
        let app = self.app.clone();
        Box::pin(async move {
            let resolved = resolve_media(app, url, audio_only, format_id).await?;
            Ok(ResolvedMediaSpec {
                download_url: resolved.download_url,
                suggested_filename: Some(resolved.suggested_filename),
                headers: resolved.headers,
            })
        })
    }

    fn download_muxed_media(
        &self,
        _task_id: String,
        url: String,
        destination_path: PathBuf,
        format_spec: String,
        cancel_flag: Arc<AtomicBool>,
        cancel_rx: watch::Receiver<bool>,
        on_progress: Arc<dyn Fn(u64, u64) + Send + Sync>,
    ) -> BoxFuture<'static, Result<PathBuf, String>> {
        let app = self.app.clone();
        Box::pin(async move {
            #[cfg(target_os = "android")]
            {
                let _ = (app, url, destination_path, format_spec, cancel_flag, cancel_rx, on_progress);
                Err("تنزيل HLS/DASH عبر yt-dlp متاح على سطح المكتب فقط حاليًا".into())
            }
            #[cfg(not(target_os = "android"))]
            {
                download_with_ytdlp_internal(
                    &app,
                    &url,
                    destination_path,
                    &format_spec,
                    cancel_flag,
                    cancel_rx,
                    on_progress,
                )
                .await
            }
        })
    }
}

#[cfg(not(target_os = "android"))]
async fn download_with_ytdlp_internal(
    app: &AppHandle,
    url: &str,
    target: PathBuf,
    format_spec: &str,
    cancel_flag: Arc<AtomicBool>,
    mut cancel_rx: watch::Receiver<bool>,
    on_progress: Arc<dyn Fn(u64, u64) + Send + Sync>,
) -> Result<PathBuf, String> {
    let (executable, node, ffmpeg) = resolve_tool_paths(app)?;
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
        format_spec,
        "--merge-output-format",
        "mp4",
        "-o",
        &target.to_string_lossy(),
    ]);
    if let Some(ff) = ffmpeg {
        if let Some(dir) = ff.parent() {
            command.args(["--ffmpeg-location", &dir.to_string_lossy()]);
        }
    }
    command.args(["--", url]);
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|e| format!("تعذر تشغيل محرك الوسائط: {e}"))?;
    if let Some(out) = child.stdout.take() {
        use tokio::io::AsyncBufReadExt;
        let mut lines = tokio::io::BufReader::new(out).lines();
        loop {
            if cancel_flag.load(std::sync::atomic::Ordering::Acquire) {
                let _ = child.kill().await;
                return Err("CANCELLED".into());
            }
            let next_line = tokio::select! {
                line = lines.next_line() => line,
                _ = argon_downloader_core::throttle::wait_cancelled(&mut cancel_rx, &cancel_flag) => {
                    let _ = child.kill().await;
                    return Err("CANCELLED".into());
                }
            };
            match next_line {
                Ok(Some(line)) => {
                    if let Some((dl, total)) = parse_ytdlp_progress(&line) {
                        on_progress(dl, total);
                    }
                }
                Ok(None) | Err(_) => break,
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
    if target.exists() {
        return Ok(target);
    }
    // yt-dlp may append .mp4 when merging separate video/audio streams
    let mp4_fallback = PathBuf::from(format!("{}.mp4", target.display()));
    if mp4_fallback.exists() {
        return Ok(mp4_fallback);
    }
    Ok(target)
}

#[tauri::command]
pub async fn media_tools_status(app: AppHandle) -> Result<MediaToolsStatus, String> {
    #[cfg(target_os = "android")]
    {
        let _ = app;
        Ok(MediaToolsStatus {
            has_ytdlp: false,
            has_node: false,
            has_ffmpeg: false,
            tools_dir: String::new(),
        })
    }
    #[cfg(not(target_os = "android"))]
    {
        let (ytdlp, node, ffmpeg) = resolve_tool_paths(&app)?;
        let tools_dir = user_tools_dir(&app)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(MediaToolsStatus {
            has_ytdlp: ytdlp.exists(),
            has_node: node.starts_with("node:") && PathBuf::from(&node[5..]).exists(),
            has_ffmpeg: ffmpeg.is_some(),
            tools_dir,
        })
    }
}

#[tauri::command]
pub async fn install_ffmpeg_tool(app: AppHandle) -> Result<MediaToolsStatus, String> {
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = app;
        return Err("التثبيت التلقائي لـ ffmpeg متاح لنظام Windows x64 حاليًا".into());
    }
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let dir = user_tools_dir(&app)?;
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| e.to_string())?;
        let dl = crate::ensure_downloader(&app).await?;
        let client = dl.http_client().clone();

        let _ = app.emit(
            "media-tool-progress",
            ToolProgressEvent {
                tool: "ffmpeg".into(),
                phase: "checksum".into(),
                downloaded: 0,
                total: None,
            },
        );

        let sha_resp = tokio::time::timeout(
            Duration::from_secs(20),
            client
                .get("https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip.sha256")
                .send(),
        )
        .await
        .map_err(|_| "انتهت مهلة جلب بصمة ffmpeg".to_string())?
        .map_err(|e| format!("تعذر الوصول إلى خادم ffmpeg: {e}"))?;
        if !sha_resp.status().is_success() {
            return Err(format!("فشل جلب بصمة ffmpeg: HTTP {}", sha_resp.status()));
        }
        let expected_sha = sha_resp
            .text()
            .await
            .map_err(|e| e.to_string())?
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if expected_sha.len() != 64 || !expected_sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("بصمة التحقق من ffmpeg غير صالحة".into());
        }

        let zip_tmp = dir.join("ffmpeg-release-essentials.tmp.zip");
        let _ = tokio::fs::remove_file(&zip_tmp).await;
        let resp = tokio::time::timeout(
            Duration::from_secs(30),
            client
                .get("https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip")
                .send(),
        )
        .await
        .map_err(|_| "انتهت مهلة بدء تنزيل ffmpeg".to_string())?
        .map_err(|e| format!("فشل تنزيل ffmpeg: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("تعذر تنزيل ffmpeg: HTTP {}", resp.status()));
        }
        let total = resp.content_length();
        let mut downloaded = 0u64;
        let mut hasher = Sha256::new();
        let mut out_file = tokio::fs::File::create(&zip_tmp)
            .await
            .map_err(|e| e.to_string())?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| format!("انقطع تنزيل ffmpeg: {e}"))?;
            hasher.update(&bytes);
            out_file
                .write_all(&bytes)
                .await
                .map_err(|e| e.to_string())?;
            downloaded += bytes.len() as u64;
            let _ = app.emit(
                "media-tool-progress",
                ToolProgressEvent {
                    tool: "ffmpeg".into(),
                    phase: "downloading".into(),
                    downloaded,
                    total,
                },
            );
        }
        out_file.sync_all().await.map_err(|e| e.to_string())?;
        drop(out_file);

        let actual_sha = format!("{:x}", hasher.finalize());
        if actual_sha != expected_sha {
            let _ = tokio::fs::remove_file(&zip_tmp).await;
            return Err("فشل التحقق من SHA-256 لأرشيف ffmpeg".into());
        }

        let _ = app.emit(
            "media-tool-progress",
            ToolProgressEvent {
                tool: "ffmpeg".into(),
                phase: "extracting".into(),
                downloaded,
                total,
            },
        );

        let dir_clone = dir.clone();
        let zip_clone = zip_tmp.clone();
        tokio::task::spawn_blocking(move || extract_ffmpeg_from_zip(&zip_clone, &dir_clone))
            .await
            .map_err(|e| e.to_string())??;
        let _ = tokio::fs::remove_file(&zip_tmp).await;

        media_tools_status(app).await
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn extract_ffmpeg_from_zip(zip_path: &PathBuf, tools_dir: &PathBuf) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("Invalid ffmpeg zip: {e}"))?;
    let mut found_exe = false;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry
            .enclosed_name()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if name.ends_with("/bin/ffmpeg.exe") {
            let tmp_exe = tools_dir.join("ffmpeg.tmp.exe");
            let mut out = std::fs::File::create(&tmp_exe).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            out.sync_all().map_err(|e| e.to_string())?;
            drop(out);
            replace_bin_atomic(&tmp_exe, &tools_dir.join("ffmpeg.exe"))?;
            found_exe = true;
        } else if name.ends_with("/LICENSE") || name.ends_with("/LICENSE.txt") {
            let mut out = std::fs::File::create(tools_dir.join("ffmpeg-LICENSE.txt"))
                .map_err(|e| e.to_string())?;
            let _ = std::io::copy(&mut entry, &mut out);
        }
    }
    if !found_exe {
        return Err("لم يُعثر على ffmpeg.exe داخل الأرشيف".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn update_ytdlp_tool(app: AppHandle) -> Result<MediaToolsStatus, String> {
    #[cfg(not(all(windows, target_arch = "x86_64")))]
    {
        let _ = app;
        return Err("التحديث المباشر لـ yt-dlp متاح لنظام Windows x64 حاليًا".into());
    }
    #[cfg(all(windows, target_arch = "x86_64"))]
    {
        let dir = user_tools_dir(&app)?;
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| e.to_string())?;
        let dl = crate::ensure_downloader(&app).await?;
        let client = dl.http_client().clone();
        let base = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";

        let sums = tokio::time::timeout(
            Duration::from_secs(20),
            client.get(format!("{base}/SHA2-256SUMS")).send(),
        )
        .await
        .map_err(|_| "انتهت مهلة جلب بصمات yt-dlp".to_string())?
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;

        let expected = sums
            .lines()
            .find_map(|line| {
                let mut parts = line.split_whitespace();
                let hash = parts.next()?;
                let name = parts.next()?.trim_start_matches('*');
                (name == "yt-dlp.exe" && hash.len() == 64).then(|| hash.to_ascii_lowercase())
            })
            .ok_or("لم تتوفر بصمة yt-dlp.exe الرسمية")?;

        let tmp_exe = dir.join("yt-dlp.tmp.exe");
        let resp = tokio::time::timeout(
            Duration::from_secs(30),
            client.get(format!("{base}/yt-dlp.exe")).send(),
        )
        .await
        .map_err(|_| "انتهت مهلة تنزيل yt-dlp".to_string())?
        .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("فشل تنزيل yt-dlp: HTTP {}", resp.status()));
        }
        let total = resp.content_length();
        let mut downloaded = 0u64;
        let mut hasher = Sha256::new();
        let mut out_file = tokio::fs::File::create(&tmp_exe)
            .await
            .map_err(|e| e.to_string())?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| e.to_string())?;
            hasher.update(&bytes);
            out_file
                .write_all(&bytes)
                .await
                .map_err(|e| e.to_string())?;
            downloaded += bytes.len() as u64;
            let _ = app.emit(
                "media-tool-progress",
                ToolProgressEvent {
                    tool: "yt-dlp".into(),
                    phase: "downloading".into(),
                    downloaded,
                    total,
                },
            );
        }
        out_file.sync_all().await.map_err(|e| e.to_string())?;
        drop(out_file);

        if format!("{:x}", hasher.finalize()) != expected {
            let _ = tokio::fs::remove_file(&tmp_exe).await;
            return Err("فشل التحقق من SHA-256 لـ yt-dlp.exe".into());
        }
        let target_exe = dir.join("yt-dlp.exe");
        tokio::task::spawn_blocking(move || replace_bin_atomic(&tmp_exe, &target_exe))
            .await
            .map_err(|e| e.to_string())??;

        media_tools_status(app).await
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn replace_bin_atomic(src: &PathBuf, dst: &PathBuf) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let s: Vec<u16> = src.as_os_str().encode_wide().chain(Some(0)).collect();
    let d: Vec<u16> = dst.as_os_str().encode_wide().chain(Some(0)).collect();
    let ok = unsafe {
        MoveFileExW(
            s.as_ptr(),
            d.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

fn from_social_bundle(bundle: social::SocialMediaBundle, allow_video_only: bool) -> MediaMetadataInfo {
    let formats = bundle
        .formats
        .into_iter()
        .filter(|f| allow_video_only || f.has_audio)
        .map(|f| MediaFormatInfo {
            id: f.id,
            ext: f.ext,
            resolution: f.resolution,
            fps: f.fps,
            filesize: f.filesize,
            vcodec: f.vcodec,
            acodec: f.acodec,
            has_video: f.has_video,
            has_audio: f.has_audio,
            label: f.label,
        })
        .collect();
    MediaMetadataInfo {
        title: bundle.title,
        uploader: bundle.uploader,
        thumbnail: bundle.thumbnail,
        duration: bundle.duration,
        has_ffmpeg: allow_video_only,
        formats,
    }
}

#[cfg(not(target_os = "android"))]
fn is_pure_rust_format_id(id: &str) -> bool {
    [
        "yt-", "ig-", "tt-", "x-", "vx-", "fb-", "reddit-", "vimeo-", "og-",
    ]
    .iter()
    .any(|prefix| id.starts_with(prefix))
}

#[tauri::command]
pub async fn list_media_formats(app: AppHandle, url: String) -> Result<MediaMetadataInfo, String> {
    let parsed = reqwest::Url::parse(&url).map_err(|_| "Invalid media URL")?;
    if !["https", "http"].contains(&parsed.scheme()) {
        return Err("Unsupported media protocol".into());
    }
    let dl = crate::ensure_downloader(&app).await?;
    let client = dl.http_client().clone();

    #[cfg(target_os = "android")]
    {
        let bundle = social::fetch_social_bundle(&client, &url).await?;
        return Ok(from_social_bundle(bundle, false));
    }

    #[cfg(not(target_os = "android"))]
    {
        let (_, _, ffmpeg) = resolve_tool_paths(&app)?;
        let has_ffmpeg = ffmpeg.is_some();
        if let Ok(value) = run_ytdlp_json(&app, &url, None).await {
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
                    let height = f["height"].as_u64();
                    let has_video = vcodec != "none"
                        || f["video_ext"].as_str().is_some_and(|e| e != "none")
                        || height.is_some();
                    let has_audio = acodec != "none"
                        || f["audio_ext"].as_str().is_some_and(|e| e != "none");
                    if !has_video && !has_audio {
                        continue;
                    }
                    let is_direct = protocol == "https" || protocol == "http";
                    if !has_ffmpeg && (!is_direct || (has_video && !has_audio)) {
                        continue;
                    }
                    let ext = f["ext"].as_str().unwrap_or("mp4").to_string();
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
            if !formats.is_empty() {
                return Ok(MediaMetadataInfo {
                    title,
                    uploader,
                    thumbnail,
                    duration,
                    has_ffmpeg,
                    formats,
                });
            }
        }
        let bundle = social::fetch_social_bundle(&client, &url).await?;
        Ok(from_social_bundle(bundle, has_ffmpeg))
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
    let dl = crate::ensure_downloader(&app).await?;
    let client = dl.http_client().clone();

    let to_resolved = |r: social::ResolvedSocialMedia| ResolvedMedia {
        download_url: r.download_url,
        suggested_filename: r.suggested_filename,
        headers: r.headers,
    };

    #[cfg(target_os = "android")]
    {
        return social::resolve_social_media(&client, &url, audio_only, format_id.as_deref())
            .await
            .map(to_resolved);
    }

    #[cfg(not(target_os = "android"))]
    {
        if format_id.as_deref().is_some_and(is_pure_rust_format_id) {
            return social::resolve_social_media(&client, &url, audio_only, format_id.as_deref())
                .await
                .map(to_resolved);
        }

        let platform = argon_downloader_core::resolver::detect_platform(&url);
        let prefer_rust_first = format_id.is_none()
            && matches!(
                platform,
                "instagram" | "tiktok" | "twitter" | "facebook" | "reddit" | "vimeo" | "threads"
            );

        if prefer_rust_first {
            if let Ok(resolved) =
                social::resolve_social_media(&client, &url, audio_only, None).await
            {
                return Ok(to_resolved(resolved));
            }
        }

        let fmt = format_id.clone().unwrap_or_else(|| {
            if audio_only {
                "bestaudio[protocol=https]/bestaudio[protocol=http]/best[protocol=https]/best[protocol=http]".to_string()
            } else {
                "best[vcodec!=none][acodec!=none][protocol=https]/best[vcodec!=none][acodec!=none][protocol=http]/best[protocol=https]/best[protocol=http]".to_string()
            }
        });

        match run_ytdlp_json(&app, &url, Some(&fmt)).await {
            Ok(value) => {
                if let Some(stream) = value["url"].as_str() {
                    if let Ok(stream_url) = reqwest::Url::parse(stream) {
                        if ["http", "https"].contains(&stream_url.scheme()) {
                            let ext = value["ext"].as_str().unwrap_or("mp4");
                            let title = argon_downloader_core::resolver::sanitize_filename(
                                value["title"].as_str().unwrap_or("media"),
                            );
                            let mut headers = HashMap::new();
                            if let Some(map) = value["http_headers"].as_object() {
                                for (key, value) in map {
                                    if let Some(value) = value.as_str() {
                                        headers.insert(key.clone(), value.to_owned());
                                    }
                                }
                            }
                            return Ok(ResolvedMedia {
                                download_url: stream.to_owned(),
                                suggested_filename: format!(
                                    "{}.{}",
                                    title.chars().take(160).collect::<String>(),
                                    ext
                                ),
                                headers,
                            });
                        }
                    }
                }
                if !prefer_rust_first {
                    if let Ok(resolved) =
                        social::resolve_social_media(&client, &url, audio_only, None).await
                    {
                        return Ok(to_resolved(resolved));
                    }
                }
                Err("لم تتوفر صيغة تنزيل مباشر لهذا الرابط".into())
            }
            Err(yt_err) => {
                if !prefer_rust_first {
                    if let Ok(resolved) =
                        social::resolve_social_media(&client, &url, audio_only, None).await
                    {
                        return Ok(to_resolved(resolved));
                    }
                }
                Err(yt_err)
            }
        }
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
