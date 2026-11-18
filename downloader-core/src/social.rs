use crate::resolver::{detect_platform, sanitize_filename};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{OnceLock, RwLock},
    time::{Duration, Instant},
};

const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";
const IPHONE_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1";
const ANDROID_YT_UA: &str = "com.google.android.youtube/20.10.38 (Linux; U; Android 14) gzip";
const IOS_YT_UA: &str = "com.google.ios.youtube/20.10.4 (iPhone16,2; U; CPU iOS 18_3_2 like Mac OS X;)";
const VR_YT_UA: &str = "com.google.android.apps.youtube.vr.oculus/1.60.19 (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SocialFormat {
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
    #[serde(skip_serializing)]
    pub url: String,
    #[serde(skip_serializing)]
    pub headers: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct SocialMediaBundle {
    pub title: String,
    pub uploader: Option<String>,
    pub thumbnail: Option<String>,
    pub duration: Option<f64>,
    pub formats: Vec<SocialFormat>,
}

#[derive(Clone, Debug)]
pub struct ResolvedSocialMedia {
    pub download_url: String,
    pub suggested_filename: String,
    pub headers: HashMap<String, String>,
}

static SOCIAL_BUNDLE_CACHE: OnceLock<RwLock<HashMap<String, (SocialMediaBundle, Instant)>>> =
    OnceLock::new();

fn get_social_cache() -> &'static RwLock<HashMap<String, (SocialMediaBundle, Instant)>> {
    SOCIAL_BUNDLE_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn get_cached_social_bundle(url: &str) -> Option<SocialMediaBundle> {
    let key = url.trim().to_owned();
    let cache = get_social_cache().read().ok()?;
    if let Some((bundle, inserted_at)) = cache.get(&key) {
        if inserted_at.elapsed() < Duration::from_secs(300) {
            return Some(bundle.clone());
        }
    }
    None
}

pub fn put_cached_social_bundle(url: &str, bundle: SocialMediaBundle) {
    let key = url.trim().to_owned();
    if let Ok(mut cache) = get_social_cache().write() {
        if cache.len() > 200 {
            cache.retain(|_, (_, inserted_at)| inserted_at.elapsed() < Duration::from_secs(300));
        }
        cache.insert(key, (bundle, Instant::now()));
    }
}

pub fn url_encode_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() * 2);
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub fn unescape_media_url(raw: &str) -> String {
    let mut s = raw.trim().to_owned();
    // Iteratively peel double-escaped JSON sequences commonly emitted by Meta/TikTok inline scripts.
    for _ in 0..2 {
        s = s
            .replace("\\\\/", "/")
            .replace("\\/", "/")
            .replace("\\\\u0026", "&")
            .replace("\\u0026", "&")
            .replace("\\u002F", "/")
            .replace("\\u002f", "/")
            .replace("\\u003D", "=")
            .replace("\\u003d", "=")
            .replace("\\u003F", "?")
            .replace("\\u003f", "?")
            .replace("\\\"", "\"");
    }
    s.replace("&amp;", "&")
        .replace("&#x26;", "&")
        .replace("&#38;", "&")
}

fn is_valid_http_url(candidate: &str) -> bool {
    Url::parse(candidate.trim())
        .ok()
        .is_some_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
}

fn push_unique_url(out: &mut Vec<String>, candidate: String) {
    if is_valid_http_url(&candidate) && !out.iter().any(|existing| existing == &candidate) {
        out.push(candidate);
    }
}

fn extract_json_text_field(haystack: &str, key: &str) -> Option<String> {
    for quoted in [format!("\"{key}\":\""), format!("\\\"{key}\\\":\\\"")] {
        if let Some(pos) = haystack.find(&quoted) {
            let start = pos + quoted.len();
            let tail = &haystack[start..];
            let end_marker = if quoted.starts_with('\\') { "\\\"" } else { "\"" };
            if let Some(end) = tail.find(end_marker) {
                let raw = tail[..end].trim();
                if !raw.is_empty() {
                    return Some(raw.to_owned());
                }
            }
        }
    }
    None
}

/// Scans HTML/JS payloads for both raw `"key":"url"` and escaped `\"key\":\"url\"` values.
pub fn extract_json_url_fields(haystack: &str, keys: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for key in keys {
        for quoted in [format!("\"{key}\":\""), format!("\\\"{key}\\\":\\\"")] {
            let mut offset = 0usize;
            while let Some(pos) = haystack[offset..].find(&quoted) {
                let start = offset + pos + quoted.len();
                let tail = &haystack[start..];
                let end_marker = if quoted.starts_with('\\') { "\\\"" } else { "\"" };
                if let Some(end) = tail.find(end_marker) {
                    let raw = &tail[..end];
                    if raw.contains("http") {
                        push_unique_url(&mut found, unescape_media_url(raw));
                    }
                    offset = start + end + end_marker.len();
                } else {
                    break;
                }
            }
        }
    }
    found
}

pub fn extract_meta_content(html: &str, attr_value: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let needle = attr_value.to_ascii_lowercase();
    let mut pos = 0usize;
    while let Some(found) = lower[pos..].find(&needle) {
        let abs = pos + found;
        let tag_start = lower[..abs].rfind("<meta")?;
        let tag_end_rel = lower[abs..].find('>')?;
        let tag = &html[tag_start..abs + tag_end_rel + 1];
        let tag_lower = &lower[tag_start..abs + tag_end_rel + 1];
        for attr in ["content=\"", "content='"] {
            if let Some(c_idx) = tag_lower.find(attr) {
                let quote = attr.as_bytes()[attr.len() - 1] as char;
                let rest = &tag[c_idx + attr.len()..];
                if let Some(end_q) = rest.find(quote) {
                    let val = unescape_media_url(&rest[..end_q]);
                    if !val.trim().is_empty() {
                        return Some(val);
                    }
                }
            }
        }
        pos = abs + needle.len();
    }
    None
}

fn extract_html_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open_end = html[start..].find('>')? + start + 1;
    let close = lower[open_end..].find("</title>")? + open_end;
    let clean = html[open_end..close]
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#064;", "@")
        .replace('\n', " ")
        .trim()
        .to_owned();
    (!clean.is_empty()).then_some(clean)
}

async fn fetch_text(
    client: &Client,
    url: &str,
    user_agent: &str,
    referer: Option<&str>,
) -> Result<(String, String), String> {
    let mut req = client
        .get(url)
        .header("User-Agent", user_agent)
        .header("Accept", "text/html,application/xhtml+xml,application/json;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Accept-Encoding", "identity");
    if let Some(r) = referer {
        req = req.header("Referer", r);
    }
    let resp = tokio::time::timeout(Duration::from_secs(18), req.send())
        .await
        .map_err(|_| "انتهت مهلة الاتصال بمنصة الوسائط".to_owned())?
        .map_err(|e| format!("تعذر الاتصال بالمنصة: {e}"))?;
    let final_url = resp.url().to_string();
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let body = tokio::time::timeout(Duration::from_secs(15), resp.text())
        .await
        .map_err(|_| "انتهت مهلة قراءة بيانات الصفحة".to_owned())?
        .map_err(|e| format!("فشل قراءة استجابة المنصة: {e}"))?;
    Ok((final_url, body))
}

pub fn extract_youtube_id(raw_url: &str) -> Option<String> {
    let url = Url::parse(raw_url.trim()).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    if host == "youtu.be" || host.ends_with(".youtu.be") {
        let seg = url.path_segments()?.next().unwrap_or("").trim();
        return (!seg.is_empty()).then(|| seg.to_owned());
    }
    if host == "youtube.com" || host.ends_with(".youtube.com") {
        if let Some((_, v)) = url.query_pairs().find(|(k, _)| k == "v") {
            if !v.is_empty() {
                return Some(v.into_owned());
            }
        }
        let segs: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
        if segs.len() >= 2 && matches!(segs[0], "shorts" | "embed" | "live" | "v") {
            return Some(segs[1].to_owned());
        }
    }
    None
}

pub fn parse_youtube_innertube_json(data: &Value) -> Result<SocialMediaBundle, String> {
    let status = data["playabilityStatus"]["status"].as_str().unwrap_or("");
    if status != "OK" {
        let reason = data["playabilityStatus"]["reason"]
            .as_str()
            .unwrap_or("الفيديو غير متاح للاستخراج المباشر");
        return Err(reason.to_owned());
    }

    let details = &data["videoDetails"];
    let title = details["title"].as_str().unwrap_or("YouTube Video").to_owned();
    let uploader = details["author"].as_str().map(str::to_owned);
    let duration = details["lengthSeconds"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok());
    let thumbnail = details["thumbnail"]["thumbnails"]
        .as_array()
        .and_then(|arr| arr.last())
        .and_then(|item| item["url"].as_str())
        .map(str::to_owned);

    let mut formats = Vec::new();
    let push_yt_fmt = |formats: &mut Vec<SocialFormat>, item: &Value, is_muxed: bool| {
        let url = if let Some(direct_url) = item["url"].as_str() {
            direct_url.to_owned()
        } else if let Some(cipher) = item["signatureCipher"].as_str().or_else(|| item["cipher"].as_str()) {
            let mut found_url = None;
            for part in cipher.split('&') {
                if let Some(val) = part.strip_prefix("url=") {
                    found_url = Some(crate::resolver::percent_decode(val));
                    break;
                }
            }
            match found_url {
                Some(u) => u,
                None => return,
            }
        } else {
            return;
        };
        if !is_valid_http_url(&url) {
            return;
        }
        let mime = item["mimeType"].as_str().unwrap_or("video/mp4");
        let base_mime = mime.split(';').next().unwrap_or("video/mp4");
        let ext = if base_mime.contains("webm") {
            "webm"
        } else if base_mime.contains("audio/mp4") {
            "m4a"
        } else if base_mime.starts_with("video/") {
            "mp4"
        } else {
            "m4a"
        };
        let has_video = is_muxed || base_mime.starts_with("video/");
        let has_audio = is_muxed || base_mime.starts_with("audio/");
        let height = item["height"].as_u64();
        let fps = item["fps"].as_u64().map(|v| v as u32);
        let bitrate_kbps = item["averageBitrate"]
            .as_u64()
            .or_else(|| item["bitrate"].as_u64())
            .map(|b| (b + 500) / 1000);
        let resolution = if has_video {
            item["qualityLabel"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| height.map(|h| format!("{h}p")))
                .unwrap_or_else(|| "Video".into())
        } else if let Some(kbps) = bitrate_kbps {
            format!("{kbps} kbps")
        } else {
            "Audio".into()
        };
        let filesize = item["contentLength"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok());
        let itag = item["itag"]
            .as_u64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| format!("{}", formats.len() + 1));

        let label = if has_video && has_audio {
            format!("{resolution} · {ext}")
        } else if has_video {
            format!("{resolution} · {ext} (video only)")
        } else if let Some(kbps) = bitrate_kbps {
            format!("Audio · {ext} · {kbps} kbps")
        } else {
            format!("Audio · {ext}")
        };

        formats.push(SocialFormat {
            id: format!("yt-{itag}"),
            ext: ext.to_owned(),
            resolution,
            fps,
            filesize,
            vcodec: if has_video { ext.to_owned() } else { "none".into() },
            acodec: if has_audio { ext.to_owned() } else { "none".into() },
            has_video,
            has_audio,
            label,
            url,
            headers: HashMap::new(),
        });
    };

    if let Some(muxed) = data["streamingData"]["formats"].as_array() {
        for item in muxed.iter().rev() {
            push_yt_fmt(&mut formats, item, true);
        }
    }
    if let Some(adaptive) = data["streamingData"]["adaptiveFormats"].as_array() {
        for item in adaptive {
            push_yt_fmt(&mut formats, item, false);
        }
    }

    if formats.is_empty() {
        return Err("لم تُعثر على صيغ بث مباشر قابلة للتنزيل من YouTube".into());
    }
    Ok(SocialMediaBundle {
        title,
        uploader,
        thumbnail,
        duration,
        formats,
    })
}

async fn extract_youtube(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    let video_id = extract_youtube_id(url).ok_or("رابط YouTube غير مكتمل")?;
    let api_url = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false";

    let clients = [
        (
            VR_YT_UA,
            json!({
                "videoId": video_id,
                "context": {
                    "client": {
                        "clientName": "ANDROID_VR",
                        "clientVersion": "1.60.19",
                        "deviceMake": "Oculus",
                        "deviceModel": "Quest 3",
                        "androidSdkVersion": 32,
                        "osName": "Android",
                        "osVersion": "12L",
                        "userAgent": VR_YT_UA,
                        "hl": "en",
                        "gl": "US"
                    }
                },
                "contentCheckOk": true,
                "racyCheckOk": true
            }),
        ),
        (
            ANDROID_YT_UA,
            json!({
                "videoId": video_id,
                "context": {
                    "client": {
                        "clientName": "ANDROID",
                        "clientVersion": "20.10.38",
                        "androidSdkVersion": 34,
                        "osName": "Android",
                        "osVersion": "14",
                        "hl": "en",
                        "gl": "US"
                    }
                },
                "contentCheckOk": true,
                "racyCheckOk": true
            }),
        ),
        (
            IOS_YT_UA,
            json!({
                "videoId": video_id,
                "context": {
                    "client": {
                        "clientName": "IOS",
                        "clientVersion": "20.10.4",
                        "deviceMake": "Apple",
                        "deviceModel": "iPhone16,2",
                        "osName": "iPhone",
                        "osVersion": "18.3.2.22D82",
                        "hl": "en",
                        "gl": "US"
                    }
                },
                "contentCheckOk": true,
                "racyCheckOk": true
            }),
        ),
    ];

    let mut adaptive_fallback: Option<SocialMediaBundle> = None;
    let mut last_err = "تعذر استخراج فيديو YouTube".to_owned();
    for (ua, body) in clients {
        let Ok(resp) = tokio::time::timeout(
            Duration::from_secs(10),
            client
                .post(api_url)
                .header("User-Agent", ua)
                .header("Content-Type", "application/json")
                .header("Accept-Encoding", "identity")
                .json(&body)
                .send(),
        )
        .await
        else {
            continue;
        };
        let Ok(resp) = resp else { continue };
        let Ok(json_val) = resp.json::<Value>().await else { continue };
        match parse_youtube_innertube_json(&json_val) {
            Ok(mut bundle) => {
                for f in &mut bundle.formats {
                    f.headers.insert("User-Agent".into(), ua.to_owned());
                }
                if bundle.formats.iter().any(|f| f.has_video && f.has_audio) {
                    return Ok(bundle);
                }
                if adaptive_fallback.is_none() {
                    adaptive_fallback = Some(bundle);
                }
            }
            Err(e) => last_err = e,
        }
    }
    adaptive_fallback.ok_or(last_err)
}

pub fn extract_instagram_shortcode(raw_url: &str) -> Option<String> {
    let url = Url::parse(raw_url.trim()).ok()?;
    let segs: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    for i in 0..segs.len() {
        if matches!(segs[i], "reel" | "reels" | "p" | "tv") && i + 1 < segs.len() {
            let code = segs[i + 1].trim().trim_matches('/');
            let clean = code.split('?').next().unwrap_or("").trim();
            if !clean.is_empty() {
                return Some(clean.to_owned());
            }
        }
        if segs[i] == "stories" && i + 2 < segs.len() {
            let code = segs[i + 2].trim().trim_matches('/');
            let clean = code.split('?').next().unwrap_or("").trim();
            if !clean.is_empty() {
                return Some(clean.to_owned());
            }
        }
    }
    None
}

pub fn parse_instagram_html(html: &str) -> Option<SocialMediaBundle> {
    let mut urls = extract_json_url_fields(
        html,
        &[
            "video_url",
            "playback_url",
            "playable_url",
            "contentUrl",
            "video_dash_manifest",
        ],
    );
    for meta_key in [
        "og:video:secure_url",
        "og:video",
        "twitter:player:stream",
    ] {
        if let Some(u) = extract_meta_content(html, meta_key) {
            push_unique_url(&mut urls, u);
        }
    }

    let mut is_photo = false;
    if urls.is_empty() {
        let img_fields = extract_json_url_fields(html, &["display_url", "display_src"]);
        for u in img_fields {
            push_unique_url(&mut urls, u);
        }
        if let Some(og_img) = extract_meta_content(html, "og:image") {
            push_unique_url(&mut urls, og_img);
        }
        if !urls.is_empty() {
            is_photo = true;
        }
    }

    if urls.is_empty() {
        return None;
    }

    let raw_title = extract_meta_content(html, "og:title")
        .or_else(|| extract_html_title(html))
        .unwrap_or_else(|| {
            if is_photo {
                "Instagram Photo".into()
            } else {
                "Instagram Video".into()
            }
        });
    let title = if raw_title.eq_ignore_ascii_case("instagram") {
        if is_photo {
            "Instagram Photo".into()
        } else {
            "Instagram Reel".into()
        }
    } else {
        raw_title
    };
    let uploader = extract_json_text_field(html, "username");
    let thumbnail = extract_meta_content(html, "og:image");

    let mut headers = HashMap::new();
    headers.insert("Referer".into(), "https://www.instagram.com/".into());
    headers.insert("User-Agent".into(), IPHONE_UA.into());

    let formats = urls
        .into_iter()
        .enumerate()
        .map(|(i, url)| {
            let ext = if is_photo { "jpg" } else { "mp4" };
            SocialFormat {
                id: format!("ig-{}", i + 1),
                ext: ext.into(),
                resolution: if is_photo {
                    "Image".into()
                } else if i == 0 {
                    "HD".into()
                } else {
                    "MP4".into()
                },
                fps: None,
                filesize: None,
                vcodec: if is_photo { "none".into() } else { "h264".into() },
                acodec: if is_photo { "none".into() } else { "aac".into() },
                has_video: !is_photo,
                has_audio: !is_photo,
                label: if is_photo {
                    format!("Photo #{}", i + 1)
                } else if i == 0 {
                    "MP4 (Original)".into()
                } else {
                    format!("MP4 #{}", i + 1)
                },
                url,
                headers: headers.clone(),
            }
        })
        .collect();

    Some(SocialMediaBundle {
        title,
        uploader,
        thumbnail,
        duration: None,
        formats,
    })
}

async fn extract_instagram(client: &Client, raw_url: &str) -> Result<SocialMediaBundle, String> {
    let mut canonical = raw_url.to_owned();
    if extract_instagram_shortcode(&canonical).is_none() {
        if let Ok((resolved, _)) = fetch_text(client, raw_url, CHROME_UA, None).await {
            if !resolved.is_empty() {
                canonical = resolved;
            }
        }
    }

    let mut candidates = Vec::new();
    if let Some(code) = extract_instagram_shortcode(&canonical) {
        candidates.push((
            format!("https://www.instagram.com/reel/{code}/embed/"),
            IPHONE_UA,
        ));
        candidates.push((
            format!("https://www.instagram.com/p/{code}/embed/"),
            IPHONE_UA,
        ));
        candidates.push((
            format!("https://www.instagram.com/p/{code}/embed/captioned/"),
            CHROME_UA,
        ));
        candidates.push((
            format!("https://www.instagram.com/reel/{code}/"),
            IPHONE_UA,
        ));
        candidates.push((
            format!("https://www.instagram.com/p/{code}/"),
            CHROME_UA,
        ));
    } else {
        candidates.push((canonical.clone(), IPHONE_UA));
    }

    for (target, ua) in candidates {
        if let Ok((_, html)) = fetch_text(client, &target, ua, Some("https://www.instagram.com/")).await {
            if let Some(bundle) = parse_instagram_html(&html) {
                return Ok(bundle);
            }
        }
    }
    Err("تعذر استخراج وسائط Instagram (قد يكون الحساب خاصاً أو يتطلب تسجيل الدخول؛ يمكنك استخدام إضافة المتصفح أو وضع ملف cookies.txt في مجلد أدوات التطبيق)".into())
}

pub fn extract_tiktok_video_id(raw_url: &str) -> Option<String> {
    let url = Url::parse(raw_url.trim()).ok()?;
    let segs: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    for i in 0..segs.len().saturating_sub(1) {
        if matches!(segs[i], "video" | "v" | "photo") {
            let code = segs[i + 1].trim();
            let clean = code.split('?').next().unwrap_or("").trim();
            if clean.chars().all(|c| c.is_ascii_digit()) && !clean.is_empty() {
                return Some(clean.to_owned());
            }
        }
    }
    for seg in segs {
        let clean = seg.split('?').next().unwrap_or("").trim();
        if clean.len() >= 15 && clean.chars().all(|c| c.is_ascii_digit()) {
            return Some(clean.to_owned());
        }
    }
    None
}

pub fn parse_tikwm_json(val: &Value) -> Option<SocialMediaBundle> {
    if val["code"].as_i64() != Some(0) {
        return None;
    }
    let d = &val["data"];
    let title = d["title"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("TikTok Video")
        .to_owned();
    let uploader = d["author"]["nickname"]
        .as_str()
        .or_else(|| d["author"]["unique_id"].as_str())
        .map(str::to_owned);
    let thumbnail = d["cover"].as_str().map(str::to_owned);
    let duration = d["duration"].as_f64();

    let mut headers = HashMap::new();
    headers.insert("User-Agent".into(), CHROME_UA.into());

    let mut formats = Vec::new();
    if let Some(hd) = d["hdplay"].as_str().filter(|s| is_valid_http_url(s)) {
        formats.push(SocialFormat {
            id: "tt-hd".into(),
            ext: "mp4".into(),
            resolution: "HD".into(),
            fps: None,
            filesize: d["hd_size"].as_u64(),
            vcodec: "h264".into(),
            acodec: "aac".into(),
            has_video: true,
            has_audio: true,
            label: "HD · mp4 (No watermark)".into(),
            url: hd.to_owned(),
            headers: headers.clone(),
        });
    }
    if let Some(play) = d["play"].as_str().filter(|s| is_valid_http_url(s)) {
        formats.push(SocialFormat {
            id: "tt-sd".into(),
            ext: "mp4".into(),
            resolution: "MP4".into(),
            fps: None,
            filesize: d["size"].as_u64(),
            vcodec: "h264".into(),
            acodec: "aac".into(),
            has_video: true,
            has_audio: true,
            label: "MP4 (No watermark)".into(),
            url: play.to_owned(),
            headers: headers.clone(),
        });
    }
    if let Some(imgs) = d["images"].as_array() {
        for (idx, img) in imgs.iter().enumerate() {
            if let Some(img_url) = img.as_str().filter(|s| is_valid_http_url(s)) {
                formats.push(SocialFormat {
                    id: format!("tt-img-{}", idx + 1),
                    ext: "jpg".into(),
                    resolution: "Image".into(),
                    fps: None,
                    filesize: None,
                    vcodec: "none".into(),
                    acodec: "none".into(),
                    has_video: false,
                    has_audio: false,
                    label: format!("Image #{}", idx + 1),
                    url: img_url.to_owned(),
                    headers: headers.clone(),
                });
            }
        }
    }
    if let Some(music) = d["music"].as_str().filter(|s| is_valid_http_url(s)) {
        formats.push(SocialFormat {
            id: "tt-audio".into(),
            ext: "mp3".into(),
            resolution: "Audio".into(),
            fps: None,
            filesize: None,
            vcodec: "none".into(),
            acodec: "mp3".into(),
            has_video: false,
            has_audio: true,
            label: "Audio · mp3".into(),
            url: music.to_owned(),
            headers,
        });
    }

    (!formats.is_empty()).then_some(SocialMediaBundle {
        title,
        uploader,
        thumbnail,
        duration,
        formats,
    })
}

async fn extract_tiktok_tikmate(client: &Client, url: &str) -> Option<SocialMediaBundle> {
    let resp = tokio::time::timeout(
        Duration::from_secs(6),
        client
            .post("https://api.tikmate.app/api/lookup")
            .header("User-Agent", CHROME_UA)
            .form(&[("url", url)])
            .send(),
    )
    .await
    .ok()?
    .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let val: Value = resp.json().await.ok()?;
    if val["success"].as_bool() != Some(true) {
        return None;
    }
    let id = val["id"].as_str().unwrap_or("").trim();
    if id.is_empty() {
        return None;
    }
    let title = val["desc"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("TikTok Video")
        .to_owned();
    let uploader = val["author_name"].as_str().map(str::to_owned);
    let thumbnail = val["cover"].as_str().map(str::to_owned);

    let mut formats = Vec::new();
    let mut headers = HashMap::new();
    headers.insert("User-Agent".into(), CHROME_UA.into());
    headers.insert("Referer".into(), "https://tikmate.app/".into());

    if let Some(hd_token) = val["hd_token"].as_str().filter(|s| !s.is_empty()) {
        let hd_url = format!("https://tikmate.app/download/{hd_token}/{id}.mp4?hd=1");
        formats.push(SocialFormat {
            id: "tt-hd".into(),
            ext: "mp4".into(),
            resolution: "HD".into(),
            fps: None,
            filesize: None,
            vcodec: "h264".into(),
            acodec: "aac".into(),
            has_video: true,
            has_audio: true,
            label: "HD · MP4 (No watermark)".into(),
            url: hd_url,
            headers: headers.clone(),
        });
    }
    if let Some(token) = val["token"].as_str().filter(|s| !s.is_empty()) {
        let sd_url = format!("https://tikmate.app/download/{token}/{id}.mp4");
        formats.push(SocialFormat {
            id: "tt-sd".into(),
            ext: "mp4".into(),
            resolution: "SD".into(),
            fps: None,
            filesize: None,
            vcodec: "h264".into(),
            acodec: "aac".into(),
            has_video: true,
            has_audio: true,
            label: "SD · MP4 (No watermark)".into(),
            url: sd_url,
            headers,
        });
    }
    (!formats.is_empty()).then_some(SocialMediaBundle {
        title,
        uploader,
        thumbnail,
        duration: None,
        formats,
    })
}

async fn extract_tiktok(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    let mut canonical = url.to_owned();
    if canonical.contains("vt.tiktok.com")
        || canonical.contains("vm.tiktok.com")
        || canonical.contains("/t/")
    {
        if let Ok((resolved, _)) = fetch_text(client, url, CHROME_UA, None).await {
            if !resolved.is_empty() {
                canonical = resolved;
            }
        }
    }

    // 1. Try TikMate primary extractor
    if let Some(bundle) = extract_tiktok_tikmate(client, &canonical).await {
        return Ok(bundle);
    }
    if canonical != url {
        if let Some(bundle) = extract_tiktok_tikmate(client, url).await {
            return Ok(bundle);
        }
    }

    // 2. Try TikWM fallback with fast timeout
    let mut tikwm_candidates = vec![canonical.clone()];
    if let Some(id) = extract_tiktok_video_id(&canonical) {
        tikwm_candidates.push(format!("https://www.tiktok.com/@tiktok/video/{id}"));
    }
    if canonical != url {
        tikwm_candidates.push(url.to_owned());
    }

    for target in tikwm_candidates {
        let api = format!(
            "https://www.tikwm.com/api/?url={}&hd=1",
            url_encode_component(&target)
        );
        let fut = fetch_text(client, &api, CHROME_UA, None);
        if let Ok(Ok((_, text))) = tokio::time::timeout(Duration::from_secs(4), fut).await {
            if let Ok(json_val) = serde_json::from_str::<Value>(&text) {
                if let Some(bundle) = parse_tikwm_json(&json_val) {
                    return Ok(bundle);
                }
            }
        }
    }

    // 3. Fallback to direct web HTML and embed scraping
    let embed_url = extract_tiktok_video_id(&canonical)
        .map(|id| format!("https://www.tiktok.com/embed/v2/{id}"));
    let targets = [Some(canonical.as_str()), embed_url.as_deref()];
    for target_url in targets.into_iter().flatten() {
        if let Ok((final_url, html)) = fetch_text(client, target_url, CHROME_UA, Some("https://www.tiktok.com/")).await {
            let mut urls = extract_json_url_fields(&html, &["playAddr", "downloadAddr", "play_addr", "download_addr"]);
            if let Some(og) = extract_meta_content(&html, "og:video") {
                push_unique_url(&mut urls, og);
            }
            if !urls.is_empty() {
                let mut headers = HashMap::new();
                headers.insert("User-Agent".into(), CHROME_UA.into());
                headers.insert("Referer".into(), final_url);

                let title = extract_meta_content(&html, "og:title")
                    .or_else(|| extract_html_title(&html))
                    .unwrap_or_else(|| "TikTok Video".into());
                return Ok(SocialMediaBundle {
                    title,
                    uploader: None,
                    thumbnail: extract_meta_content(&html, "og:image"),
                    duration: None,
                    formats: urls
                        .into_iter()
                        .enumerate()
                        .map(|(i, stream_url)| SocialFormat {
                            id: format!("tt-web-{}", i + 1),
                            ext: "mp4".into(),
                            resolution: "MP4".into(),
                            fps: None,
                            filesize: None,
                            vcodec: "h264".into(),
                            acodec: "aac".into(),
                            has_video: true,
                            has_audio: true,
                            label: if i == 0 {
                                "MP4".into()
                            } else {
                                format!("MP4 #{}", i + 1)
                            },
                            url: stream_url,
                            headers: headers.clone(),
                        })
                        .collect(),
                });
            }
        }
    }

    Err("تعذر استخراج فيديو TikTok (تأكد من صحة الرابط أو استخدم إضافة المتصفح)".into())
}

pub fn extract_tweet_id(raw_url: &str) -> Option<String> {
    let url = Url::parse(raw_url.trim()).ok()?;
    let segs: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    for i in 0..segs.len().saturating_sub(1) {
        if segs[i] == "status" {
            let id = segs[i + 1];
            if id.bytes().all(|b| b.is_ascii_digit()) {
                return Some(id.to_owned());
            }
        }
    }
    None
}

pub fn parse_fxtwitter_json(val: &Value) -> Option<SocialMediaBundle> {
    let tweet = &val["tweet"];
    if tweet.is_null() {
        return None;
    }
    let mut formats = Vec::new();
    let mut thumbnail = None;
    let mut duration = None;

    if let Some(videos) = tweet["media"]["videos"].as_array() {
        if let Some(first_vid) = videos.first() {
            thumbnail = first_vid["thumbnail_url"].as_str().map(str::to_owned);
            duration = first_vid["duration"].as_f64();
        }
        for (idx, vid) in videos.iter().enumerate() {
            if let Some(variants) = vid["variants"].as_array() {
                let mut mp4s: Vec<&Value> = variants
                    .iter()
                    .filter(|v| {
                        v["content_type"].as_str() == Some("video/mp4")
                            && v["url"].as_str().is_some_and(is_valid_http_url)
                    })
                    .collect();
                mp4s.sort_by_key(|v| std::cmp::Reverse(v["bitrate"].as_u64().unwrap_or(0)));
                for (v_idx, var) in mp4s.iter().enumerate() {
                    if let Some(u) = var["url"].as_str() {
                        formats.push(SocialFormat {
                            id: format!("x-{idx}-{v_idx}"),
                            ext: "mp4".into(),
                            resolution: if v_idx == 0 { "HD" } else { "SD" }.into(),
                            fps: None,
                            filesize: None,
                            vcodec: "h264".into(),
                            acodec: "aac".into(),
                            has_video: true,
                            has_audio: true,
                            label: if v_idx == 0 {
                                "HD · mp4".into()
                            } else {
                                format!("MP4 (Variant {})", v_idx + 1)
                            },
                            url: u.to_owned(),
                            headers: HashMap::new(),
                        });
                    }
                }
            }
            if let Some(u) = vid["url"].as_str().filter(|u| is_valid_http_url(u)) {
                if !formats.iter().any(|f| f.url == u) {
                    formats.push(SocialFormat {
                        id: format!("x-direct-{idx}"),
                        ext: "mp4".into(),
                        resolution: "HD".into(),
                        fps: None,
                        filesize: None,
                        vcodec: "h264".into(),
                        acodec: "aac".into(),
                        has_video: true,
                        has_audio: true,
                        label: "MP4".into(),
                        url: u.to_owned(),
                        headers: HashMap::new(),
                    });
                }
            }
        }
    }

    if let Some(photos) = tweet["media"]["photos"].as_array() {
        if thumbnail.is_none() {
            thumbnail = photos.first().and_then(|p| p["url"].as_str()).map(str::to_owned);
        }
        for (idx, p) in photos.iter().enumerate() {
            if let Some(u) = p["url"].as_str().filter(|u| is_valid_http_url(u)) {
                formats.push(SocialFormat {
                    id: format!("x-photo-{idx}"),
                    ext: "jpg".into(),
                    resolution: "Photo".into(),
                    fps: None,
                    filesize: None,
                    vcodec: "none".into(),
                    acodec: "none".into(),
                    has_video: false,
                    has_audio: false,
                    label: if photos.len() == 1 { "Photo · jpg".into() } else { format!("Photo #{} · jpg", idx + 1) },
                    url: u.to_owned(),
                    headers: HashMap::new(),
                });
            }
        }
    }

    if formats.is_empty() {
        return None;
    }

    Some(SocialMediaBundle {
        title: tweet["text"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.chars().take(80).collect())
            .unwrap_or_else(|| "X Post".into()),
        uploader: tweet["author"]["name"].as_str().map(str::to_owned),
        thumbnail,
        duration,
        formats,
    })
}

async fn extract_twitter_twitsave(client: &Client, tweet_id: &str) -> Option<SocialMediaBundle> {
    let url = format!("https://twitsave.com/info?url=https://twitter.com/i/status/{tweet_id}");
    let (_, html) = fetch_text(client, &url, CHROME_UA, None).await.ok()?;
    let mut urls = extract_json_url_fields(&html, &["url", "download"]);
    for meta in ["og:video", "og:video:secure_url"] {
        if let Some(u) = extract_meta_content(&html, meta) {
            push_unique_url(&mut urls, u);
        }
    }
    let lower = html.to_ascii_lowercase();
    let mut offset = 0;
    while let Some(pos) = lower[offset..].find("https://video.twimg.com/") {
        let start = offset + pos;
        let tail = &html[start..];
        let end = tail.find(['"', '\'', ' ', '<', '>']).unwrap_or(tail.len());
        let candidate = unescape_media_url(&tail[..end]);
        if is_valid_http_url(&candidate) {
            push_unique_url(&mut urls, candidate);
        }
        offset = start + end.max(1);
    }
    if urls.is_empty() {
        return None;
    }
    let title = extract_meta_content(&html, "og:title")
        .or_else(|| extract_html_title(&html))
        .unwrap_or_else(|| "Twitter Video".into());
    Some(SocialMediaBundle {
        title,
        uploader: None,
        thumbnail: extract_meta_content(&html, "og:image"),
        duration: None,
        formats: urls
            .into_iter()
            .enumerate()
            .map(|(i, u)| SocialFormat {
                id: format!("x-twitsave-{i}"),
                ext: "mp4".into(),
                resolution: if i == 0 { "HD" } else { "SD" }.into(),
                fps: None,
                filesize: None,
                vcodec: "h264".into(),
                acodec: "aac".into(),
                has_video: true,
                has_audio: true,
                label: if i == 0 { "HD · mp4".into() } else { format!("MP4 #{}", i + 1) },
                url: u,
                headers: HashMap::new(),
            })
            .collect(),
    })
}

async fn extract_twitter(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    let tweet_id = extract_tweet_id(url).ok_or("لم يُعثر على معرف المنشور في رابط X/Twitter")?;

    let fx_candidates = [
        format!("https://api.fxtwitter.com/status/{tweet_id}"),
        format!("https://api.fixupx.com/status/{tweet_id}"),
        format!("https://api.fxtwitter.com/i/status/{tweet_id}"),
    ];

    for fx_url in fx_candidates {
        if let Ok((_, txt)) = fetch_text(client, &fx_url, "ARGON/2.3", None).await {
            if let Ok(val) = serde_json::from_str::<Value>(&txt) {
                if let Some(bundle) = parse_fxtwitter_json(&val) {
                    return Ok(bundle);
                }
            }
        }
    }

    let vx_url = format!("https://api.vxtwitter.com/Twitter/status/{tweet_id}");
    if let Ok((_, vx_txt)) = fetch_text(client, &vx_url, "ARGON/2.3", None).await {
        if let Ok(val) = serde_json::from_str::<Value>(&vx_txt) {
            let mut formats = Vec::new();
            if let Some(list) = val["media_extended"].as_array() {
                for (i, item) in list.iter().enumerate() {
                    if matches!(item["type"].as_str(), Some("video" | "gif")) {
                        if let Some(u) = item["url"].as_str().filter(|u| is_valid_http_url(u)) {
                            formats.push(SocialFormat {
                                id: format!("vx-{i}"),
                                ext: "mp4".into(),
                                resolution: "MP4".into(),
                                fps: None,
                                filesize: None,
                                vcodec: "h264".into(),
                                acodec: "aac".into(),
                                has_video: true,
                                has_audio: true,
                                label: "MP4".into(),
                                url: u.to_owned(),
                                headers: HashMap::new(),
                            });
                        }
                    } else if item["type"].as_str() == Some("image") {
                        if let Some(u) = item["url"].as_str().filter(|u| is_valid_http_url(u)) {
                            formats.push(SocialFormat {
                                id: format!("vx-img-{i}"),
                                ext: "jpg".into(),
                                resolution: "Photo".into(),
                                fps: None,
                                filesize: None,
                                vcodec: "none".into(),
                                acodec: "none".into(),
                                has_video: false,
                                has_audio: false,
                                label: format!("Photo #{}", i + 1),
                                url: u.to_owned(),
                                headers: HashMap::new(),
                            });
                        }
                    }
                }
            }
            if !formats.is_empty() {
                return Ok(SocialMediaBundle {
                    title: val["text"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .map(|s| s.chars().take(80).collect())
                        .unwrap_or_else(|| "X Post".into()),
                    uploader: val["user_name"].as_str().map(str::to_owned),
                    thumbnail: None,
                    duration: None,
                    formats,
                });
            }
        }
    }

    if let Some(bundle) = extract_twitter_twitsave(client, &tweet_id).await {
        return Ok(bundle);
    }

    Err("لا يحتوي هذا المنشور على وسائط أو يتعذر الوصول إليه".into())
}

async fn extract_facebook(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    let mut urls = Vec::new();
    let mut title = None;
    let mut thumbnail = None;

    // 1. Query Facebook's official video plugin embed endpoint.
    // Zero login redirect wall for public videos, reels, and watch clips!
    let plugin_url = format!(
        "https://www.facebook.com/plugins/video.php?href={}&show_text=0",
        url_encode_component(url)
    );
    if let Ok((_, p_html)) = fetch_text(client, &plugin_url, CHROME_UA, Some("https://www.facebook.com/")).await {
        let p_urls = extract_json_url_fields(
            &p_html,
            &[
                "hd_src",
                "sd_src",
                "hd_src_no_ratelimit",
                "sd_src_no_ratelimit",
                "browser_native_hd_url",
                "playable_url_quality_hd",
                "browser_native_sd_url",
                "playable_url",
                "video_url",
            ],
        );
        for u in p_urls {
            push_unique_url(&mut urls, u);
        }
        title = extract_meta_content(&p_html, "og:title").or_else(|| extract_html_title(&p_html));
        thumbnail = extract_meta_content(&p_html, "og:image");
    }

    // 2. Try desktop HTML if plugin didn't yield video URLs
    if urls.is_empty() {
        if let Ok((_, html)) = fetch_text(client, url, CHROME_UA, Some("https://www.facebook.com/")).await {
            let d_urls = extract_json_url_fields(
                &html,
                &[
                    "browser_native_hd_url",
                    "playable_url_quality_hd",
                    "hd_src",
                    "browser_native_sd_url",
                    "playable_url",
                    "sd_src",
                    "video_url",
                ],
            );
            for u in d_urls {
                push_unique_url(&mut urls, u);
            }
            for meta in ["og:video:secure_url", "og:video"] {
                if let Some(u) = extract_meta_content(&html, meta) {
                    push_unique_url(&mut urls, u);
                }
            }
            if title.is_none() {
                title = extract_meta_content(&html, "og:title").or_else(|| extract_html_title(&html));
            }
            if thumbnail.is_none() {
                thumbnail = extract_meta_content(&html, "og:image");
            }
        }
    }

    // 3. Try mobile HTML if desktop had no video URLs
    if urls.is_empty() {
        let mobile_url = url
            .replace("www.facebook.com", "m.facebook.com")
            .replace("web.facebook.com", "m.facebook.com");
        if let Ok((_, m_html)) = fetch_text(client, &mobile_url, IPHONE_UA, Some("https://m.facebook.com/")).await {
            let m_urls = extract_json_url_fields(
                &m_html,
                &[
                    "browser_native_hd_url",
                    "playable_url_quality_hd",
                    "hd_src",
                    "browser_native_sd_url",
                    "playable_url",
                    "sd_src",
                    "video_url",
                ],
            );
            for u in m_urls {
                push_unique_url(&mut urls, u);
            }
            for meta in ["og:video:secure_url", "og:video"] {
                if let Some(u) = extract_meta_content(&m_html, meta) {
                    push_unique_url(&mut urls, u);
                }
            }
            if title.is_none() {
                title = extract_meta_content(&m_html, "og:title").or_else(|| extract_html_title(&m_html));
            }
            if thumbnail.is_none() {
                thumbnail = extract_meta_content(&m_html, "og:image");
            }
        }
    }

    let mut formats = Vec::new();
    let mut headers = HashMap::new();
    headers.insert("User-Agent".into(), CHROME_UA.into());
    headers.insert("Referer".into(), "https://www.facebook.com/".into());

    for (i, u) in urls.into_iter().enumerate() {
        formats.push(SocialFormat {
            id: format!("fb-{}", i + 1),
            ext: "mp4".into(),
            resolution: if i == 0 { "HD" } else { "SD" }.into(),
            fps: None,
            filesize: None,
            vcodec: "h264".into(),
            acodec: "aac".into(),
            has_video: true,
            has_audio: true,
            label: if i == 0 { "HD · mp4".into() } else { "SD · mp4".into() },
            url: u,
            headers: headers.clone(),
        });
    }

    // 4. Fallback for photo posts
    if formats.is_empty() {
        if let Some(ref img_url) = thumbnail {
            if is_valid_http_url(img_url) {
                formats.push(SocialFormat {
                    id: "fb-img-1".into(),
                    ext: "jpg".into(),
                    resolution: "Photo".into(),
                    fps: None,
                    filesize: None,
                    vcodec: "none".into(),
                    acodec: "none".into(),
                    has_video: false,
                    has_audio: false,
                    label: "Photo · jpg".into(),
                    url: img_url.clone(),
                    headers: headers.clone(),
                });
            }
        }
    }

    if formats.is_empty() {
        return Err("تعذر استخراج فيديو Facebook (قد يتطلب تسجيل الدخول أو حساب عام)".into());
    }

    Ok(SocialMediaBundle {
        title: title.unwrap_or_else(|| "Facebook Video".into()),
        uploader: None,
        thumbnail,
        duration: None,
        formats,
    })
}

async fn extract_reddit(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    let (resolved, _) = fetch_text(client, url, CHROME_UA, None).await?;
    let mut parsed = Url::parse(&resolved).map_err(|_| "Invalid Reddit URL")?;
    parsed.set_query(None);
    parsed.set_fragment(None);
    let base = parsed.to_string().trim_end_matches('/').to_owned();
    let json_url = if base.ends_with(".json") {
        base
    } else {
        format!("{base}.json?raw_json=1")
    };
    let (_, raw) = fetch_text(client, &json_url, "ARGON/2.3 (Media Downloader)", None).await?;
    let val: Value = serde_json::from_str(&raw).map_err(|_| "تعذر قراءة بيانات Reddit")?;
    let post = &val[0]["data"]["children"][0]["data"];

    let title = post["title"]
        .as_str()
        .unwrap_or("Reddit Post")
        .chars()
        .take(100)
        .collect();
    let uploader = post["author"].as_str().map(str::to_owned);
    let mut formats = Vec::new();

    // 1. Check for video
    let rv = if !post["secure_media"]["reddit_video"].is_null() {
        &post["secure_media"]["reddit_video"]
    } else if !post["crosspost_parent_list"][0]["secure_media"]["reddit_video"].is_null() {
        &post["crosspost_parent_list"][0]["secure_media"]["reddit_video"]
    } else {
        &post["media"]["reddit_video"]
    };
    if let Some(fallback) = rv["fallback_url"].as_str().filter(|u| is_valid_http_url(u)) {
        formats.push(SocialFormat {
            id: "reddit-mp4".into(),
            ext: "mp4".into(),
            resolution: rv["height"]
                .as_u64()
                .map(|h| format!("{h}p"))
                .unwrap_or_else(|| "MP4".into()),
            fps: None,
            filesize: None,
            vcodec: "h264".into(),
            acodec: "aac".into(),
            has_video: true,
            has_audio: rv["has_audio"].as_bool().unwrap_or(false),
            label: "MP4".into(),
            url: fallback.to_owned(),
            headers: HashMap::new(),
        });
    }

    // 2. Check for gallery
    if let Some(media_meta) = post["media_metadata"].as_object() {
        for (idx, (_key, item)) in media_meta.iter().enumerate() {
            if let Some(u) = item["s"]["u"].as_str().filter(|u| is_valid_http_url(u)) {
                let clean_u = unescape_media_url(u);
                formats.push(SocialFormat {
                    id: format!("reddit-gallery-{idx}"),
                    ext: "jpg".into(),
                    resolution: "Photo".into(),
                    fps: None,
                    filesize: None,
                    vcodec: "none".into(),
                    acodec: "none".into(),
                    has_video: false,
                    has_audio: false,
                    label: format!("Image #{}", idx + 1),
                    url: clean_u,
                    headers: HashMap::new(),
                });
            }
        }
    }

    // 3. Check for single image
    if formats.is_empty() {
        if let Some(post_url) = post["url"].as_str().filter(|u| is_valid_http_url(u)) {
            let lower = post_url.to_ascii_lowercase();
            if lower.ends_with(".jpg") || lower.ends_with(".png") || lower.ends_with(".webp") || lower.ends_with(".gif") {
                let ext = lower.split('.').last().unwrap_or("jpg").to_owned();
                formats.push(SocialFormat {
                    id: "reddit-image".into(),
                    ext,
                    resolution: "Photo".into(),
                    fps: None,
                    filesize: None,
                    vcodec: "none".into(),
                    acodec: "none".into(),
                    has_video: false,
                    has_audio: false,
                    label: "Photo".into(),
                    url: post_url.to_owned(),
                    headers: HashMap::new(),
                });
            }
        }
    }

    if formats.is_empty() {
        return Err("لا يحتوي منشور Reddit هذا على وسائط قابلة للتنزيل".into());
    }

    let thumbnail = post["thumbnail"].as_str().filter(|u| is_valid_http_url(u)).map(str::to_owned);

    Ok(SocialMediaBundle {
        title,
        uploader,
        thumbnail,
        duration: rv["duration"].as_f64(),
        formats,
    })
}

async fn extract_pinterest(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    let (_final_url, html) = fetch_text(client, url, CHROME_UA, Some("https://www.pinterest.com/")).await?;
    let mut formats = Vec::new();

    let mut video_urls = extract_json_url_fields(&html, &["url", "videoUrl"]);
    for meta in ["og:video", "og:video:secure_url"] {
        if let Some(u) = extract_meta_content(&html, meta) {
            push_unique_url(&mut video_urls, u);
        }
    }
    for (i, u) in video_urls.into_iter().filter(|u| u.contains(".mp4") || u.contains("v.pinimg.com")).enumerate() {
        formats.push(SocialFormat {
            id: format!("pin-video-{}", i + 1),
            ext: "mp4".into(),
            resolution: "MP4".into(),
            fps: None,
            filesize: None,
            vcodec: "h264".into(),
            acodec: "aac".into(),
            has_video: true,
            has_audio: true,
            label: if i == 0 { "Video · mp4".into() } else { format!("Video #{} · mp4", i + 1) },
            url: u,
            headers: HashMap::new(),
        });
    }

    let mut image_urls = Vec::new();
    for meta in ["og:image", "og:image:secure_url"] {
        if let Some(u) = extract_meta_content(&html, meta) {
            push_unique_url(&mut image_urls, u);
        }
    }
    for (i, u) in image_urls.into_iter().enumerate() {
        formats.push(SocialFormat {
            id: format!("pin-img-{}", i + 1),
            ext: "jpg".into(),
            resolution: "Image".into(),
            fps: None,
            filesize: None,
            vcodec: "none".into(),
            acodec: "none".into(),
            has_video: false,
            has_audio: false,
            label: if i == 0 { "Image · jpg".into() } else { format!("Image #{} · jpg", i + 1) },
            url: u,
            headers: HashMap::new(),
        });
    }

    if formats.is_empty() {
        return Err("لم يُعثر على وسائط قابلة للتنزيل من Pinterest".into());
    }

    let title = extract_meta_content(&html, "og:title")
        .or_else(|| extract_html_title(&html))
        .unwrap_or_else(|| "Pinterest Pin".into());

    Ok(SocialMediaBundle {
        title,
        uploader: None,
        thumbnail: extract_meta_content(&html, "og:image"),
        duration: None,
        formats,
    })
}

async fn extract_vimeo(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    let parsed = Url::parse(url.trim()).map_err(|_| "Invalid Vimeo URL")?;
    let clip_id = parsed
        .path_segments()
        .and_then(|mut s| s.rfind(|seg| !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_digit())))
        .ok_or("لم يُعثر على رقم فيديو Vimeo")?;
    let cfg_url = format!("https://player.vimeo.com/video/{clip_id}/config");
    let (_, raw) = fetch_text(client, &cfg_url, CHROME_UA, Some("https://vimeo.com/")).await?;
    let val: Value = serde_json::from_str(&raw).map_err(|_| "تعذر قراءة إعدادات Vimeo")?;
    let mut files = val["request"]["files"]["progressive"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    files.sort_by_key(|f| std::cmp::Reverse(f["height"].as_u64().unwrap_or(0)));
    let formats: Vec<SocialFormat> = files
        .into_iter()
        .filter_map(|f| {
            let u = f["url"].as_str()?.to_owned();
            if !is_valid_http_url(&u) {
                return None;
            }
            let q = f["quality"].as_str().unwrap_or("MP4").to_owned();
            Some(SocialFormat {
                id: format!("vimeo-{q}"),
                ext: "mp4".into(),
                resolution: q.clone(),
                fps: f["fps"].as_f64().map(|v| v.round() as u32),
                filesize: None,
                vcodec: "h264".into(),
                acodec: "aac".into(),
                has_video: true,
                has_audio: true,
                label: format!("{q} · mp4"),
                url: u,
                headers: HashMap::new(),
            })
        })
        .collect();
    if formats.is_empty() {
        return Err("لم تتوفر صيغ MP4 مباشرة من Vimeo".into());
    }
    Ok(SocialMediaBundle {
        title: val["video"]["title"]
            .as_str()
            .unwrap_or("Vimeo Video")
            .to_owned(),
        uploader: val["video"]["owner"]["name"].as_str().map(str::to_owned),
        thumbnail: None,
        duration: val["video"]["duration"].as_f64(),
        formats,
    })
}

async fn extract_generic_html(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    let (_, html) = fetch_text(client, url, CHROME_UA, None).await?;
    let mut urls = extract_json_url_fields(&html, &["video_url", "contentUrl"]);
    for meta in ["og:video:secure_url", "og:video", "twitter:player:stream"] {
        if let Some(u) = extract_meta_content(&html, meta) {
            push_unique_url(&mut urls, u);
        }
    }
    if urls.is_empty() {
        return Err("لم يتوفر رابط فيديو مباشر قابل للاستخراج من هذه الصفحة".into());
    }
    Ok(SocialMediaBundle {
        title: extract_meta_content(&html, "og:title")
            .or_else(|| extract_html_title(&html))
            .unwrap_or_else(|| "Social Video".into()),
        uploader: None,
        thumbnail: extract_meta_content(&html, "og:image"),
        duration: None,
        formats: urls
            .into_iter()
            .enumerate()
            .map(|(i, u)| SocialFormat {
                id: format!("og-{}", i + 1),
                ext: "mp4".into(),
                resolution: "MP4".into(),
                fps: None,
                filesize: None,
                vcodec: "h264".into(),
                acodec: "aac".into(),
                has_video: true,
                has_audio: true,
                label: "MP4".into(),
                url: u,
                headers: HashMap::new(),
            })
            .collect(),
    })
}

async fn extract_threads(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    let (final_url, html) = fetch_text(client, url, CHROME_UA, Some("https://www.threads.net/")).await?;
    let mut urls = extract_json_url_fields(
        &html,
        &[
            "video_url",
            "playback_url",
            "browser_native_hd_url",
            "browser_native_sd_url",
        ],
    );
    for meta in ["og:video:secure_url", "og:video", "twitter:player:stream"] {
        if let Some(u) = extract_meta_content(&html, meta) {
            push_unique_url(&mut urls, u);
        }
    }

    let title = extract_meta_content(&html, "og:title")
        .or_else(|| extract_html_title(&html))
        .unwrap_or_else(|| "Threads Post".into());
    let thumbnail = extract_meta_content(&html, "og:image");

    let mut formats = Vec::new();
    let mut headers = HashMap::new();
    headers.insert("User-Agent".into(), CHROME_UA.into());
    headers.insert("Referer".into(), "https://www.threads.net/".into());

    for (i, u) in urls.into_iter().enumerate() {
        formats.push(SocialFormat {
            id: format!("threads-{}", i + 1),
            ext: "mp4".into(),
            resolution: if i == 0 { "HD" } else { "SD" }.into(),
            fps: None,
            filesize: None,
            vcodec: "h264".into(),
            acodec: "aac".into(),
            has_video: true,
            has_audio: true,
            label: if i == 0 { "HD · mp4".into() } else { "SD · mp4".into() },
            url: u,
            headers: headers.clone(),
        });
    }

    if formats.is_empty() {
        if let Some(ref img) = thumbnail {
            if is_valid_http_url(img) && !img.contains("rsrc.php") {
                formats.push(SocialFormat {
                    id: "threads-img-1".into(),
                    ext: "jpg".into(),
                    resolution: "Photo".into(),
                    fps: None,
                    filesize: None,
                    vcodec: "none".into(),
                    acodec: "none".into(),
                    has_video: false,
                    has_audio: false,
                    label: "Photo · jpg".into(),
                    url: img.clone(),
                    headers: headers.clone(),
                });
            }
        }
    }

    if formats.is_empty() {
        return extract_generic_html(client, &final_url).await;
    }

    Ok(SocialMediaBundle {
        title,
        uploader: None,
        thumbnail,
        duration: None,
        formats,
    })
}

async fn extract_streamable(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    let parsed = Url::parse(url.trim()).map_err(|e| e.to_string())?;
    let shortcode = parsed
        .path_segments()
        .and_then(|mut s| s.next())
        .filter(|s| !s.is_empty())
        .ok_or("رابط Streamable غير صالح")?;

    let api_url = format!("https://api.streamable.com/videos/{shortcode}");
    let (_, body) = fetch_text(client, &api_url, CHROME_UA, Some("https://streamable.com/")).await?;
    let data: Value = serde_json::from_str(&body).map_err(|e| format!("فشل تحليل استجابة Streamable: {e}"))?;

    let title = data["title"].as_str().unwrap_or("Streamable Video").to_owned();
    let thumbnail = data["thumbnail_url"].as_str().map(str::to_owned);

    let mut formats = Vec::new();
    let mut headers = HashMap::new();
    headers.insert("User-Agent".into(), CHROME_UA.into());
    headers.insert("Referer".into(), "https://streamable.com/".into());

    if let Some(files) = data["files"].as_object() {
        for (key, file_val) in files {
            if let Some(raw_url) = file_val["url"].as_str() {
                let mut clean_url = raw_url.trim().to_owned();
                if clean_url.starts_with("//") {
                    clean_url = format!("https:{clean_url}");
                } else if clean_url.starts_with("https:https://") {
                    clean_url = clean_url.replacen("https:https://", "https://", 1);
                }
                if is_valid_http_url(&clean_url) {
                    let height = file_val["height"].as_u64();
                    let resolution = height.map(|h| format!("{h}p")).unwrap_or_else(|| key.to_uppercase());
                    let filesize = file_val["size"].as_u64();
                    formats.push(SocialFormat {
                        id: format!("streamable-{key}"),
                        ext: "mp4".into(),
                        resolution: resolution.clone(),
                        fps: file_val["framerate"].as_f64().map(|f| f.round() as u32),
                        filesize,
                        vcodec: "h264".into(),
                        acodec: "aac".into(),
                        has_video: true,
                        has_audio: true,
                        label: format!("{resolution} · mp4"),
                        url: clean_url,
                        headers: headers.clone(),
                    });
                }
            }
        }
    }

    if formats.is_empty() {
        return extract_generic_html(client, url).await;
    }

    formats.sort_by(|a, b| {
        let h_a = a.resolution.trim_end_matches('p').parse::<u32>().unwrap_or(0);
        let h_b = b.resolution.trim_end_matches('p').parse::<u32>().unwrap_or(0);
        h_b.cmp(&h_a)
    });

    Ok(SocialMediaBundle {
        title,
        uploader: None,
        thumbnail,
        duration: None,
        formats,
    })
}

async fn extract_snapchat(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    let (final_url, html) = fetch_text(client, url, CHROME_UA, Some("https://www.snapchat.com/")).await?;
    let mut urls = extract_json_url_fields(
        &html,
        &[
            "mediaUrl",
            "contentUrl",
            "video_url",
            "downloadUrl",
        ],
    );
    for meta in ["og:video:secure_url", "og:video", "twitter:player:stream"] {
        if let Some(u) = extract_meta_content(&html, meta) {
            push_unique_url(&mut urls, u);
        }
    }

    let title = extract_meta_content(&html, "og:title")
        .or_else(|| extract_html_title(&html))
        .unwrap_or_else(|| "Snapchat Spotlight".into());
    let thumbnail = extract_meta_content(&html, "og:image");

    let mut formats = Vec::new();
    let mut headers = HashMap::new();
    headers.insert("User-Agent".into(), CHROME_UA.into());
    headers.insert("Referer".into(), "https://www.snapchat.com/".into());

    for (i, u) in urls.into_iter().enumerate() {
        formats.push(SocialFormat {
            id: format!("snap-{}", i + 1),
            ext: "mp4".into(),
            resolution: "HD".into(),
            fps: None,
            filesize: None,
            vcodec: "h264".into(),
            acodec: "aac".into(),
            has_video: true,
            has_audio: true,
            label: "HD · mp4".into(),
            url: u,
            headers: headers.clone(),
        });
    }

    if formats.is_empty() {
        if let Some(ref img) = thumbnail {
            if is_valid_http_url(img) {
                formats.push(SocialFormat {
                    id: "snap-img-1".into(),
                    ext: "jpg".into(),
                    resolution: "Photo".into(),
                    fps: None,
                    filesize: None,
                    vcodec: "none".into(),
                    acodec: "none".into(),
                    has_video: false,
                    has_audio: false,
                    label: "Photo · jpg".into(),
                    url: img.clone(),
                    headers: headers.clone(),
                });
            }
        }
    }

    if formats.is_empty() {
        return extract_generic_html(client, &final_url).await;
    }

    Ok(SocialMediaBundle {
        title,
        uploader: None,
        thumbnail,
        duration: None,
        formats,
    })
}

async fn fetch_social_bundle_uncached(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    match detect_platform(url) {
        "youtube" => extract_youtube(client, url).await,
        "instagram" => extract_instagram(client, url).await,
        "tiktok" => extract_tiktok(client, url).await,
        "twitter" => extract_twitter(client, url).await,
        "facebook" => extract_facebook(client, url).await,
        "reddit" => extract_reddit(client, url).await,
        "pinterest" => extract_pinterest(client, url).await,
        "vimeo" => extract_vimeo(client, url).await,
        "threads" => extract_threads(client, url).await,
        "streamable" => extract_streamable(client, url).await,
        "snapchat" => extract_snapchat(client, url).await,
        "soundcloud" | "other" => extract_generic_html(client, url).await,
        _ => extract_generic_html(client, url).await,
    }
}

pub async fn fetch_social_bundle(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    if let Some(cached) = get_cached_social_bundle(url) {
        return Ok(cached);
    }
    let bundle = fetch_social_bundle_uncached(client, url).await?;
    put_cached_social_bundle(url, bundle.clone());
    Ok(bundle)
}

pub fn pick_best_format<'a>(
    bundle: &'a SocialMediaBundle,
    audio_only: bool,
    format_id: Option<&str>,
) -> Option<&'a SocialFormat> {
    if let Some(id) = format_id {
        if let Some(exact) = bundle.formats.iter().find(|f| f.id == id) {
            return Some(exact);
        }
    }
    if audio_only {
        let audios: Vec<&SocialFormat> = bundle.formats.iter().filter(|f| !f.has_video && f.has_audio).collect();
        if let Some(best_audio) = audios.iter().max_by_key(|f| f.filesize.unwrap_or(0)) {
            return Some(*best_audio);
        }
        if let Some(audio) = audios.first() {
            return Some(*audio);
        }
    }
    let muxed: Vec<&SocialFormat> = bundle.formats.iter().filter(|f| f.has_video && f.has_audio).collect();
    if let Some(best_muxed) = muxed.iter().max_by_key(|f| f.filesize.unwrap_or(0)) {
        return Some(*best_muxed);
    }
    let videos: Vec<&SocialFormat> = bundle.formats.iter().filter(|f| f.has_video).collect();
    if let Some(best_video) = videos.iter().max_by_key(|f| f.filesize.unwrap_or(0)) {
        return Some(*best_video);
    }
    bundle.formats.first()
}

pub async fn resolve_social_media(
    client: &Client,
    url: &str,
    audio_only: bool,
    format_id: Option<&str>,
) -> Result<ResolvedSocialMedia, String> {
    let bundle = fetch_social_bundle(client, url).await?;
    let chosen = pick_best_format(&bundle, audio_only, format_id)
        .ok_or("لم تتوفر صيغة صالحة للتنزيل")?;
    let stem: String = sanitize_filename(&bundle.title).chars().take(160).collect();
    Ok(ResolvedSocialMedia {
        download_url: chosen.url.clone(),
        suggested_filename: format!("{stem}.{}", chosen.ext),
        headers: chosen.headers.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescapes_meta_and_html_urls_cleanly() {
        let escaped = r#"https:\/\/scontent.cdninstagram.com\/v\/clip.mp4?a=1\u0026b=2&amp;c=3"#;
        assert_eq!(
            unescape_media_url(escaped),
            "https://scontent.cdninstagram.com/v/clip.mp4?a=1&b=2&c=3"
        );
        let triple = r#"https:\\\/\\\/instagram.fna.fbcdn.net\\\/clip.mp4?a=1\\u0026b=2"#;
        assert_eq!(
            unescape_media_url(triple),
            "https://instagram.fna.fbcdn.net/clip.mp4?a=1&b=2"
        );
    }

    #[test]
    fn parses_instagram_embed_double_escaped_video_url() {
        let html = r#"
            <html><head><title>Instagram</title><meta property="og:image" content="https://cdn/thumb.jpg"></head>
            <body><script>{"owner":{"username":"akram"},"video_url":"https:\/\/instagram.fna.fbcdn.net\/v\/clip.mp4?x=1\u0026y=2"}</script>
            </body></html>"#;
        let parsed = parse_instagram_html(html).expect("ig embed should parse");
        assert_eq!(
            parsed.formats[0].url,
            "https://instagram.fna.fbcdn.net/v/clip.mp4?x=1&y=2"
        );
        assert_eq!(parsed.uploader.as_deref(), Some("akram"));
        assert_eq!(
            parsed.formats[0].headers.get("Referer").map(String::as_str),
            Some("https://www.instagram.com/")
        );
    }

    #[test]
    fn parses_youtube_innertube_response_and_picks_muxed_video() {
        let payload = json!({
            "playabilityStatus": { "status": "OK" },
            "videoDetails": {
                "title": "Test YouTube Clip",
                "author": "Creator",
                "lengthSeconds": "95",
                "thumbnail": { "thumbnails": [{ "url": "https://i.ytimg.com/vi/abc/hq.jpg" }] }
            },
            "streamingData": {
                "formats": [
                    { "itag": 18, "url": "https://r1.googlevideo.com/v360.mp4", "mimeType": "video/mp4; codecs=\"avc1\"", "qualityLabel": "360p", "height": 360, "contentLength": "1000000" },
                    { "itag": 22, "url": "https://r1.googlevideo.com/v720.mp4", "mimeType": "video/mp4; codecs=\"avc1\"", "qualityLabel": "720p", "height": 720, "contentLength": "3000000" }
                ],
                "adaptiveFormats": [
                    { "itag": 140, "url": "https://r1.googlevideo.com/audio.m4a", "mimeType": "audio/mp4; codecs=\"mp4a\"", "contentLength": "400000" }
                ]
            }
        });
        let bundle = parse_youtube_innertube_json(&payload).unwrap();
        assert_eq!(bundle.title, "Test YouTube Clip");
        assert_eq!(
            pick_best_format(&bundle, false, None).unwrap().url,
            "https://r1.googlevideo.com/v720.mp4"
        );
        assert_eq!(
            pick_best_format(&bundle, true, None).unwrap().url,
            "https://r1.googlevideo.com/audio.m4a"
        );
    }

    #[test]
    fn parses_tikwm_and_fxtwitter_payloads() {
        let tikwm = json!({
            "code": 0,
            "data": {
                "title": "Cool Clip",
                "hdplay": "https://v16.tiktokcdn.com/hd.mp4",
                "play": "https://v16.tiktokcdn.com/sd.mp4",
                "music": "https://v16.tiktokcdn.com/audio.mp3",
                "hd_size": 4500000,
                "size": 1800000,
                "duration": 14,
                "author": { "nickname": "tk" }
            }
        });
        let tt = parse_tikwm_json(&tikwm).unwrap();
        assert_eq!(tt.formats.len(), 3);
        assert_eq!(tt.formats[0].url, "https://v16.tiktokcdn.com/hd.mp4");

        let fx = json!({
            "tweet": {
                "text": "Post clip",
                "author": { "name": "UserX" },
                "media": {
                    "videos": [{
                        "duration": 9.5,
                        "url": "https://video.twimg.com/low.mp4",
                        "variants": [
                            { "content_type": "video/mp4", "bitrate": 832000, "url": "https://video.twimg.com/med.mp4" },
                            { "content_type": "video/mp4", "bitrate": 2176000, "url": "https://video.twimg.com/hd.mp4" }
                        ]
                    }]
                }
            }
        });
        let x = parse_fxtwitter_json(&fx).unwrap();
        assert_eq!(x.formats[0].url, "https://video.twimg.com/hd.mp4");

        let tikwm_images = json!({
            "code": 0,
            "data": {
                "title": "Photo Slideshow",
                "images": [
                    "https://p16.tiktokcdn.com/img1.jpg",
                    "https://p16.tiktokcdn.com/img2.jpg"
                ],
                "music": "https://v16.tiktokcdn.com/audio.mp3",
                "author": { "nickname": "photo_creator" }
            }
        });
        let tt_img = parse_tikwm_json(&tikwm_images).unwrap();
        assert_eq!(tt_img.formats.len(), 3);
        assert_eq!(tt_img.formats[0].ext, "jpg");
        assert_eq!(tt_img.formats[0].label, "Image #1");
        assert_eq!(tt_img.formats[2].ext, "mp3");

        let fx_photos = json!({
            "tweet": {
                "text": "Photo tweet",
                "author": { "name": "Photographer" },
                "media": {
                    "photos": [
                        { "url": "https://pbs.twimg.com/media/photo1.jpg" },
                        { "url": "https://pbs.twimg.com/media/photo2.jpg" }
                    ]
                }
            }
        });
        let x_photo = parse_fxtwitter_json(&fx_photos).unwrap();
        assert_eq!(x_photo.formats.len(), 2);
        assert_eq!(x_photo.formats[0].ext, "jpg");
        assert_eq!(x_photo.formats[0].url, "https://pbs.twimg.com/media/photo1.jpg");

        let yt_cipher_payload = json!({
            "playabilityStatus": { "status": "OK" },
            "videoDetails": {
                "title": "Cipher Clip",
                "author": "Cipher Creator",
                "lengthSeconds": "60",
                "thumbnail": { "thumbnails": [{ "url": "https://i.ytimg.com/vi/cipher/hq.jpg" }] }
            },
            "streamingData": {
                "formats": [
                    { "itag": 18, "signatureCipher": "s=abc&sp=sig&url=https%3A%2F%2Fr1.googlevideo.com%2Fcipher360.mp4", "mimeType": "video/mp4; codecs=\"avc1\"", "qualityLabel": "360p", "height": 360 }
                ]
            }
        });
        let yt_cipher = parse_youtube_innertube_json(&yt_cipher_payload).unwrap();
        assert_eq!(yt_cipher.formats[0].url, "https://r1.googlevideo.com/cipher360.mp4");

        assert_eq!(
            extract_instagram_shortcode("https://www.instagram.com/reel/C8r_6oIuGZz/"),
            Some("C8r_6oIuGZz".into())
        );
        assert_eq!(
            extract_instagram_shortcode("https://www.instagram.com/share/reel/C8r_6oIuGZz/?igsh=123"),
            Some("C8r_6oIuGZz".into())
        );
        assert_eq!(
            extract_tiktok_video_id("https://www.tiktok.com/@user/video/7106594312292453675?is_from_webapp=1"),
            Some("7106594312292453675".into())
        );
    }
}
