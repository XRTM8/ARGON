use crate::resolver::{detect_platform, sanitize_filename};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, time::Duration};

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
        let Some(url) = item["url"].as_str() else {
            return;
        };
        if !is_valid_http_url(url) {
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
        let resolution = item["qualityLabel"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| height.map(|h| format!("{h}p")))
            .unwrap_or_else(|| if has_video { "Video".into() } else { "Audio".into() });
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
            url: url.to_owned(),
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
            Duration::from_secs(16),
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
    for i in 0..segs.len().saturating_sub(1) {
        if matches!(segs[i], "reel" | "reels" | "p" | "tv") {
            let code = segs[i + 1].trim();
            if !code.is_empty() && !code.starts_with('?') {
                return Some(code.to_owned());
            }
        }
    }
    None
}

pub fn parse_instagram_html(html: &str) -> Option<SocialMediaBundle> {
    let mut urls = extract_json_url_fields(
        html,
        &["video_url", "playback_url", "playable_url", "contentUrl"],
    );
    for meta_key in ["og:video:secure_url", "og:video"] {
        if let Some(u) = extract_meta_content(html, meta_key) {
            push_unique_url(&mut urls, u);
        }
    }
    if urls.is_empty() {
        return None;
    }

    let raw_title = extract_meta_content(html, "og:title")
        .or_else(|| extract_html_title(html))
        .unwrap_or_else(|| "Instagram Video".into());
    let title = if raw_title.eq_ignore_ascii_case("instagram") {
        "Instagram Reel".into()
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
        .map(|(i, url)| SocialFormat {
            id: format!("ig-{}", i + 1),
            ext: "mp4".into(),
            resolution: if i == 0 { "HD" } else { "MP4" }.into(),
            fps: None,
            filesize: None,
            vcodec: "h264".into(),
            acodec: "aac".into(),
            has_video: true,
            has_audio: true,
            label: if i == 0 {
                "MP4 (Original)".into()
            } else {
                format!("MP4 #{}", i + 1)
            },
            url,
            headers: headers.clone(),
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
            canonical = resolved;
        }
    }

    let mut candidates = Vec::new();
    if let Some(code) = extract_instagram_shortcode(&canonical) {
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
    Err("تعذر استخراج فيديو Instagram (قد يكون الحساب خاصاً أو الرابط منتهياً)".into())
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

async fn extract_tiktok(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    let api = format!(
        "https://www.tikwm.com/api/?url={}&hd=1",
        url_encode_component(url)
    );
    if let Ok((_, text)) = fetch_text(client, &api, CHROME_UA, None).await {
        if let Ok(json_val) = serde_json::from_str::<Value>(&text) {
            if let Some(bundle) = parse_tikwm_json(&json_val) {
                return Ok(bundle);
            }
        }
    }

    let (final_url, html) = fetch_text(client, url, CHROME_UA, Some("https://www.tiktok.com/")).await?;
    let mut urls = extract_json_url_fields(&html, &["playAddr", "downloadAddr"]);
    if let Some(og) = extract_meta_content(&html, "og:video") {
        push_unique_url(&mut urls, og);
    }
    if urls.is_empty() {
        return Err("تعذر استخراج فيديو TikTok".into());
    }
    let mut headers = HashMap::new();
    headers.insert("User-Agent".into(), CHROME_UA.into());
    headers.insert("Referer".into(), final_url);

    let title = extract_meta_content(&html, "og:title")
        .or_else(|| extract_html_title(&html))
        .unwrap_or_else(|| "TikTok Video".into());
    Ok(SocialMediaBundle {
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
    })
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
    let videos = tweet["media"]["videos"].as_array()?;
    let mut formats = Vec::new();
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
    (!formats.is_empty()).then(|| SocialMediaBundle {
        title: tweet["text"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.chars().take(80).collect())
            .unwrap_or_else(|| "X Video".into()),
        uploader: tweet["author"]["name"].as_str().map(str::to_owned),
        thumbnail: videos[0]["thumbnail_url"].as_str().map(str::to_owned),
        duration: videos[0]["duration"].as_f64(),
        formats,
    })
}

async fn extract_twitter(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    let tweet_id = extract_tweet_id(url).ok_or("لم يُعثر على معرف المنشور في رابط X/Twitter")?;
    let fx_url = format!("https://api.fxtwitter.com/i/status/{tweet_id}");
    if let Ok((_, txt)) = fetch_text(client, &fx_url, "ARGON/2.3", None).await {
        if let Ok(val) = serde_json::from_str::<Value>(&txt) {
            if let Some(bundle) = parse_fxtwitter_json(&val) {
                return Ok(bundle);
            }
        }
    }

    let vx_url = format!("https://api.vxtwitter.com/Twitter/status/{tweet_id}");
    let (_, vx_txt) = fetch_text(client, &vx_url, "ARGON/2.3", None).await?;
    let val: Value = serde_json::from_str(&vx_txt).map_err(|_| "استجابة غير صالحة من X/Twitter")?;
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
            }
        }
    }
    if formats.is_empty() {
        return Err("لا يحتوي هذا المنشور على فيديو".into());
    }
    Ok(SocialMediaBundle {
        title: val["text"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.chars().take(80).collect())
            .unwrap_or_else(|| "X Video".into()),
        uploader: val["user_name"].as_str().map(str::to_owned),
        thumbnail: None,
        duration: None,
        formats,
    })
}

async fn extract_facebook(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    let (_, html) = fetch_text(client, url, CHROME_UA, Some("https://www.facebook.com/")).await?;
    let mut urls = extract_json_url_fields(
        &html,
        &[
            "browser_native_hd_url",
            "playable_url_quality_hd",
            "hd_src",
            "browser_native_sd_url",
            "playable_url",
            "sd_src",
        ],
    );
    for meta in ["og:video:secure_url", "og:video"] {
        if let Some(u) = extract_meta_content(&html, meta) {
            push_unique_url(&mut urls, u);
        }
    }
    if urls.is_empty() {
        return Err("تعذر استخراج فيديو Facebook (قد يتطلب تسجيل الدخول)".into());
    }
    let mut headers = HashMap::new();
    headers.insert("User-Agent".into(), CHROME_UA.into());
    headers.insert("Referer".into(), "https://www.facebook.com/".into());
    Ok(SocialMediaBundle {
        title: extract_meta_content(&html, "og:title")
            .or_else(|| extract_html_title(&html))
            .unwrap_or_else(|| "Facebook Video".into()),
        uploader: None,
        thumbnail: extract_meta_content(&html, "og:image"),
        duration: None,
        formats: urls
            .into_iter()
            .enumerate()
            .map(|(i, u)| SocialFormat {
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
            })
            .collect(),
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
    let rv = if !post["secure_media"]["reddit_video"].is_null() {
        &post["secure_media"]["reddit_video"]
    } else {
        &post["crosspost_parent_list"][0]["secure_media"]["reddit_video"]
    };
    let fallback = rv["fallback_url"]
        .as_str()
        .filter(|u| is_valid_http_url(u))
        .ok_or("لا يحتوي منشور Reddit هذا على فيديو")?;
    Ok(SocialMediaBundle {
        title: post["title"]
            .as_str()
            .unwrap_or("Reddit Video")
            .chars()
            .take(100)
            .collect(),
        uploader: post["author"].as_str().map(str::to_owned),
        thumbnail: None,
        duration: rv["duration"].as_f64(),
        formats: vec![SocialFormat {
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
        }],
    })
}

async fn extract_vimeo(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    let parsed = Url::parse(url.trim()).map_err(|_| "Invalid Vimeo URL")?;
    let clip_id = parsed
        .path_segments()
        .and_then(|s| s.filter(|seg| !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_digit())).next_back())
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

pub async fn fetch_social_bundle(client: &Client, url: &str) -> Result<SocialMediaBundle, String> {
    match detect_platform(url) {
        "youtube" => extract_youtube(client, url).await,
        "instagram" => extract_instagram(client, url).await,
        "tiktok" => extract_tiktok(client, url).await,
        "twitter" => extract_twitter(client, url).await,
        "facebook" => extract_facebook(client, url).await,
        "reddit" => extract_reddit(client, url).await,
        "vimeo" => extract_vimeo(client, url).await,
        "threads" | "soundcloud" | "other" => extract_generic_html(client, url).await,
        _ => extract_generic_html(client, url).await,
    }
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
        if let Some(audio) = bundle.formats.iter().find(|f| !f.has_video && f.has_audio) {
            return Some(audio);
        }
    }
    bundle
        .formats
        .iter()
        .find(|f| f.has_video && f.has_audio)
        .or_else(|| bundle.formats.first())
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
    }
}
