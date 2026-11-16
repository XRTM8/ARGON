use crate::model::{ChunkStatus, DownloadChunk, TaskCategory};
use reqwest::Url;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct ResolvedLink {
    pub resolved_url: String,
    pub original_url: String,
    pub suggested_headers: HashMap<String, String>,
    pub service_name: Option<String>,
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

pub fn percent_decode(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let mut chars = input.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(c1), Some(c2)) = (h1, h2) {
                let hex_str = [c1, c2];
                if let Ok(val) = std::str::from_utf8(&hex_str) {
                    if let Ok(byte) = u8::from_str_radix(val, 16) {
                        bytes.push(byte);
                        continue;
                    }
                }
                bytes.push(b'%');
                bytes.push(c1);
                bytes.push(c2);
            } else {
                bytes.push(b'%');
                if let Some(c1) = h1 {
                    bytes.push(c1);
                }
            }
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

pub fn sanitize_filename(raw: &str) -> String {
    let clean_name = raw
        .replace('\\', "/")
        .split('/')
        .rfind(|s| !s.is_empty() && *s != "." && *s != "..")
        .unwrap_or("download")
        .to_string();

    let mut sanitized: String = clean_name
        .chars()
        .map(|c| {
            if matches!(
                c,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'..='\x1f'
            ) {
                '_'
            } else {
                c
            }
        })
        .take(200)
        .collect();

    let trimmed = sanitized.trim().trim_matches(|c| c == '.' || c == ' ');
    if trimmed.is_empty() {
        return "download".to_string();
    }
    sanitized = trimmed.to_string();

    let stem = sanitized
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    let is_reserved = matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );

    if is_reserved {
        format!("_{sanitized}")
    } else {
        sanitized
    }
}

pub fn split_filename(filename: &str) -> (String, String) {
    let clean = filename.trim();
    const COMPOUND: &[&str] = &[
        ".tar.gz",
        ".tar.bz2",
        ".tar.xz",
        ".tar.zst",
        ".user.js",
        ".user.css",
    ];
    let lower = clean.to_ascii_lowercase();
    for ext in COMPOUND {
        if lower.ends_with(ext) {
            let cut = clean.len() - ext.len();
            let stem = &clean[..cut];
            return (
                if stem.is_empty() {
                    "file".to_owned()
                } else {
                    stem.to_owned()
                },
                clean[cut..].to_owned(),
            );
        }
    }
    if let Some(dot) = clean.rfind('.') {
        if dot > 0 && dot + 1 < clean.len() {
            return (clean[..dot].to_owned(), clean[dot..].to_owned());
        }
    }
    (
        if clean.is_empty() {
            "download".to_owned()
        } else {
            clean.to_owned()
        },
        String::new(),
    )
}

pub fn deduplicate_filename(desired: &str, existing: &[String]) -> String {
    let used: HashSet<String> = existing.iter().map(|s| s.to_ascii_lowercase()).collect();
    let clean = sanitize_filename(desired);
    if !used.contains(&clean.to_ascii_lowercase()) {
        return clean;
    }
    let (stem, ext) = split_filename(&clean);
    for idx in 1..10_000 {
        let candidate = format!("{stem} ({idx}){ext}");
        if !used.contains(&candidate.to_ascii_lowercase()) {
            return candidate;
        }
    }
    format!("{stem}_{}{ext}", crate::model::now_ms())
}

pub fn parse_content_disposition_filename(cd: &str) -> Option<String> {
    let lower = cd.to_ascii_lowercase();
    if let Some(pos) = lower.find("filename*=") {
        let after = &cd[pos + 10..];
        let raw_val = after.split(';').next().unwrap_or("").trim();
        let encoded_val = raw_val
            .strip_prefix("UTF-8''")
            .or_else(|| raw_val.strip_prefix("utf-8''"))
            .unwrap_or(raw_val);
        let decoded = percent_decode(encoded_val);
        let s = decoded.trim_matches(|c| c == '"' || c == '\'');
        if !s.is_empty() {
            return Some(sanitize_filename(s));
        }
    }
    if let Some(pos) = lower.find("filename=") {
        let after = &cd[pos + 9..];
        let raw_val = after.split(';').next().unwrap_or("").trim();
        let clean = raw_val.trim_matches(|c| c == '"' || c == '\'');
        if !clean.is_empty() {
            return Some(sanitize_filename(clean));
        }
    }
    None
}

pub fn extract_filename_from_url(raw_url: &str) -> String {
    if let Ok(parsed) = Url::parse(raw_url) {
        if let Some(last) = parsed
            .path_segments()
            .and_then(|mut segs| segs.rfind(|s| !s.is_empty()))
        {
            let decoded = percent_decode(last);
            if !decoded.trim().is_empty() {
                return sanitize_filename(&decoded);
            }
        }
    }
    format!("download_{}", crate::model::now_ms())
}

pub fn detect_category(filename: &str) -> TaskCategory {
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "torrent" => TaskCategory::Torrents,
        "exe" | "msi" | "apk" | "dmg" | "deb" | "rpm" | "iso" | "img" | "bin" => {
            TaskCategory::Programs
        }
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "mp3" | "flac" | "wav" | "aac" | "opus"
        | "ogg" | "m4a" | "m4v" | "flv" | "wmv" | "3gp" | "jpg" | "jpeg" | "png" | "webp"
        | "gif" | "svg" | "avif" | "bmp" | "ico" => TaskCategory::Media,
        "pdf" | "docx" | "doc" | "xlsx" | "xls" | "pptx" | "ppt" | "txt" | "csv" | "epub"
        | "md" => TaskCategory::Documents,
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" => TaskCategory::Archives,
        _ => TaskCategory::Other,
    }
}

pub fn is_stream_manifest(raw_url: &str) -> bool {
    Url::parse(raw_url).is_ok_and(|url| {
        let path = url.path().to_ascii_lowercase();
        path.ends_with(".m3u8") || path.ends_with(".mpd")
    })
}

pub fn detect_platform(raw_url: &str) -> &'static str {
    let Ok(url) = Url::parse(raw_url.trim()) else {
        return "other";
    };
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    let matches = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    if matches("youtube.com") || matches("youtu.be") {
        return "youtube";
    }
    if matches("tiktok.com") {
        return "tiktok";
    }
    if matches("instagram.com") || matches("instagr.am") || matches("ig.me") {
        return "instagram";
    }
    if matches("twitter.com") || matches("x.com") || matches("t.co") {
        return "twitter";
    }
    if matches("facebook.com") || matches("fb.watch") || matches("fb.com") {
        return "facebook";
    }
    if matches("reddit.com") || matches("redd.it") || matches("v.redd.it") {
        return "reddit";
    }
    if matches("threads.net") || matches("threads.com") {
        return "threads";
    }
    if matches("vimeo.com") {
        return "vimeo";
    }
    if matches("soundcloud.com") {
        return "soundcloud";
    }
    if matches("pinterest.com") || matches("pin.it") || host.contains("pinterest.") {
        return "pinterest";
    }
    if matches("snapchat.com") {
        return "snapchat";
    }
    if matches("linkedin.com") {
        return "linkedin";
    }
    if matches("twitch.tv") {
        return "twitch";
    }
    if matches("kick.com") {
        return "kick";
    }
    if matches("dailymotion.com") || matches("dai.ly") {
        return "dailymotion";
    }
    if matches("bilibili.com") || matches("b23.tv") {
        return "bilibili";
    }
    if matches("kuaishou.com") || matches("kwai.com") {
        return "kwai";
    }
    if matches("douyin.com") {
        return "douyin";
    }
    if matches("bsky.app") {
        return "bluesky";
    }
    if matches("tumblr.com") {
        return "tumblr";
    }
    if matches("streamable.com") {
        return "streamable";
    }
    if matches("loom.com") {
        return "loom";
    }
    if matches("vk.com") || matches("vkvideo.ru") {
        return "vk";
    }
    if matches("rumble.com") {
        return "rumble";
    }
    if matches("odysee.com") {
        return "odysee";
    }
    if matches("bandcamp.com") {
        return "bandcamp";
    }
    let path = url.path().to_ascii_lowercase();
    if [
        ".mp4", ".mkv", ".webm", ".avi", ".mov", ".flv", ".ts", ".mp3", ".m4a", ".aac", ".flac",
        ".wav", ".ogg", ".opus",
    ]
    .iter()
    .any(|ext| path.ends_with(ext))
    {
        "direct"
    } else {
        "other"
    }
}

pub fn resolve_universal_link(raw_url: &str) -> ResolvedLink {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return ResolvedLink {
            resolved_url: String::new(),
            original_url: raw_url.to_owned(),
            suggested_headers: HashMap::new(),
            service_name: None,
        };
    }

    let Ok(mut url) = Url::parse(trimmed) else {
        return ResolvedLink {
            resolved_url: trimmed.to_owned(),
            original_url: trimmed.to_owned(),
            suggested_headers: HashMap::new(),
            service_name: None,
        };
    };

    let mut suggested_headers = HashMap::new();
    if !url.username().is_empty() || url.password().is_some() {
        let user = percent_decode(url.username());
        let pass = url.password().map(percent_decode).unwrap_or_default();
        let creds = format!("{user}:{pass}");
        suggested_headers.insert(
            "Authorization".to_owned(),
            format!("Basic {}", base64_encode(creds.as_bytes())),
        );
        let _ = url.set_username("");
        let _ = url.set_password(None);
    }

    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    let path = url.path().to_string();

    if host == "drive.google.com" || host.ends_with(".drive.google.com") {
        if let Some(rest) = path.split("/file/d/").nth(1) {
            if let Some(id) = rest.split('/').next().filter(|s| !s.is_empty()) {
                return ResolvedLink {
                    resolved_url: format!("https://drive.google.com/uc?export=download&id={id}"),
                    original_url: trimmed.to_owned(),
                    suggested_headers,
                    service_name: Some("Google Drive".to_owned()),
                };
            }
        }
        if let Some((_, id)) = url.query_pairs().find(|(k, _)| k == "id") {
            return ResolvedLink {
                resolved_url: format!("https://drive.google.com/uc?export=download&id={id}"),
                original_url: trimmed.to_owned(),
                suggested_headers,
                service_name: Some("Google Drive".to_owned()),
            };
        }
    }

    if host == "dropbox.com" || host.ends_with(".dropbox.com") {
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(k, _)| k != "dl")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        url.query_pairs_mut()
            .clear()
            .extend_pairs(pairs)
            .append_pair("dl", "1");
        return ResolvedLink {
            resolved_url: url.to_string(),
            original_url: trimmed.to_owned(),
            suggested_headers,
            service_name: Some("Dropbox".to_owned()),
        };
    }

    if host == "github.com" && path.contains("/blob/") {
        let _ = url.set_host(Some("raw.githubusercontent.com"));
        url.set_path(&path.replacen("/blob/", "/", 1));
        return ResolvedLink {
            resolved_url: url.to_string(),
            original_url: trimmed.to_owned(),
            suggested_headers,
            service_name: Some("GitHub Raw".to_owned()),
        };
    }

    if (host == "huggingface.co" || host.ends_with(".huggingface.co")) && path.contains("/blob/") {
        url.set_path(&path.replacen("/blob/", "/resolve/", 1));
        return ResolvedLink {
            resolved_url: url.to_string(),
            original_url: trimmed.to_owned(),
            suggested_headers,
            service_name: Some("Hugging Face".to_owned()),
        };
    }

    if (host == "archive.org" || host.ends_with(".archive.org")) && path.contains("/details/") {
        url.set_path(&path.replacen("/details/", "/download/", 1));
        return ResolvedLink {
            resolved_url: url.to_string(),
            original_url: trimmed.to_owned(),
            suggested_headers,
            service_name: Some("Internet Archive".to_owned()),
        };
    }

    if let Some(origin_host) = url.host_str() {
        let port = url
            .port()
            .map(|p| format!(":{p}"))
            .unwrap_or_default();
        suggested_headers.insert(
            "Referer".to_owned(),
            format!("{}://{origin_host}{port}/", url.scheme()),
        );
    }

    ResolvedLink {
        resolved_url: url.to_string(),
        original_url: trimmed.to_owned(),
        suggested_headers,
        service_name: None,
    }
}

pub fn calculate_segments(total_bytes: u64, segment_count: u32) -> Vec<DownloadChunk> {
    if total_bytes == 0 {
        return vec![DownloadChunk {
            index: 0,
            start_byte: 0,
            end_byte: 0,
            downloaded: 0,
            status: ChunkStatus::Pending,
            speed_bytes_per_sec: None,
        }];
    }
    let requested = segment_count.clamp(1, 32) as u64;
    if requested <= 1 || total_bytes < 512 {
        return vec![DownloadChunk {
            index: 0,
            start_byte: 0,
            end_byte: total_bytes - 1,
            downloaded: 0,
            status: ChunkStatus::Pending,
            speed_bytes_per_sec: None,
        }];
    }
    let count = requested.min((total_bytes / 128).max(1));
    let chunk_size = total_bytes / count;
    (0..count)
        .map(|i| {
            let start_byte = i * chunk_size;
            let end_byte = if i == count - 1 {
                total_bytes - 1
            } else {
                (i + 1) * chunk_size - 1
            };
            DownloadChunk {
                index: i as u32,
                start_byte,
                end_byte,
                downloaded: 0,
                status: ChunkStatus::Pending,
                speed_bytes_per_sec: None,
            }
        })
        .collect()
}

/// Recommends an optimal connection / segment count based on file size and user preference.
/// Mitigates connection overhead for tiny files while maximizing throughput on large payloads.
pub fn recommended_segment_count(total_bytes: u64, preferred: u32) -> u32 {
    let preferred = preferred.clamp(1, 32);
    if total_bytes == 0 {
        return preferred.min(4);
    }
    if total_bytes < 1024 * 1024 {
        1
    } else if total_bytes < 10 * 1024 * 1024 {
        preferred.min(4)
    } else if total_bytes < 50 * 1024 * 1024 {
        preferred.min(8)
    } else {
        preferred.max(8).min(32)
    }
}

pub fn same_resource_url(a: &str, b: &str) -> bool {
    let Ok(ua) = Url::parse(a.trim()) else {
        return false;
    };
    let Ok(ub) = Url::parse(b.trim()) else {
        return false;
    };
    ua.scheme() == ub.scheme()
        && ua.host_str() == ub.host_str()
        && ua.port_or_known_default() == ub.port_or_known_default()
        && ua.path() == ub.path()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_special_hosts_and_basic_auth() {
        let auth = resolve_universal_link("https://user:p%40ss@example.com/archive.zip");
        assert_eq!(auth.resolved_url, "https://example.com/archive.zip");
        assert_eq!(
            auth.suggested_headers.get("Authorization").map(String::as_str),
            Some("Basic dXNlcjpwQHNz")
        );

        let gh = resolve_universal_link("https://github.com/org/repo/blob/main/data.bin");
        assert_eq!(
            gh.resolved_url,
            "https://raw.githubusercontent.com/org/repo/main/data.bin"
        );

        let gd = resolve_universal_link("https://drive.google.com/file/d/abc123_XYZ/view");
        assert_eq!(
            gd.resolved_url,
            "https://drive.google.com/uc?export=download&id=abc123_XYZ"
        );
    }

    #[test]
    fn sanitizes_and_deduplicates_filenames_with_compound_extensions() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("CON.txt"), "_CON.txt");
        let existing = vec!["backup.tar.gz".to_owned(), "backup (1).tar.gz".to_owned()];
        assert_eq!(
            deduplicate_filename("backup.tar.gz", &existing),
            "backup (2).tar.gz"
        );
    }

    #[test]
    fn detects_all_supported_social_platforms() {
        assert_eq!(detect_platform("https://www.youtube.com/watch?v=123"), "youtube");
        assert_eq!(detect_platform("https://youtu.be/123"), "youtube");
        assert_eq!(detect_platform("https://www.tiktok.com/@u/video/123"), "tiktok");
        assert_eq!(detect_platform("https://www.instagram.com/reel/123/"), "instagram");
        assert_eq!(detect_platform("https://x.com/u/status/123"), "twitter");
        assert_eq!(detect_platform("https://t.co/xyz123"), "twitter");
        assert_eq!(detect_platform("https://www.facebook.com/reel/123"), "facebook");
        assert_eq!(detect_platform("https://fb.watch/123"), "facebook");
        assert_eq!(detect_platform("https://www.reddit.com/r/rust/comments/123/"), "reddit");
        assert_eq!(detect_platform("https://v.redd.it/123"), "reddit");
        assert_eq!(detect_platform("https://www.pinterest.com/pin/123/"), "pinterest");
        assert_eq!(detect_platform("https://pin.it/123"), "pinterest");
        assert_eq!(detect_platform("https://www.snapchat.com/spotlight/123"), "snapchat");
        assert_eq!(detect_platform("https://www.twitch.tv/videos/123"), "twitch");
        assert_eq!(detect_platform("https://kick.com/video/123"), "kick");
        assert_eq!(detect_platform("https://www.bilibili.com/video/123"), "bilibili");
        assert_eq!(detect_platform("https://bsky.app/profile/u/post/123"), "bluesky");
        assert_eq!(detect_platform("https://example.com/movie.mp4"), "direct");
        assert_eq!(detect_platform("https://example.com/installer.iso"), "other");
    }

    #[test]
    fn detects_categories_reliably() {
        assert_eq!(detect_category("song.opus"), TaskCategory::Media);
        assert_eq!(detect_category("track.m4a"), TaskCategory::Media);
        assert_eq!(detect_category("photo.avif"), TaskCategory::Media);
        assert_eq!(detect_category("film.m4v"), TaskCategory::Media);
        assert_eq!(detect_category("stream.flv"), TaskCategory::Media);
        assert_eq!(detect_category("video.wmv"), TaskCategory::Media);
        assert_eq!(detect_category("clip.3gp"), TaskCategory::Media);
        assert_eq!(detect_category("icon.ico"), TaskCategory::Media);
        assert_eq!(detect_category("image.bmp"), TaskCategory::Media);
        assert_eq!(detect_category("setup.exe"), TaskCategory::Programs);
        assert_eq!(detect_category("paper.pdf"), TaskCategory::Documents);
        assert_eq!(detect_category("backup.tar.gz"), TaskCategory::Archives);
        assert_eq!(detect_category("archive.7z"), TaskCategory::Archives);
        assert_eq!(detect_category("ubuntu.iso.torrent"), TaskCategory::Torrents);
        assert_eq!(detect_category("file.unknown_xyz"), TaskCategory::Other);
    }

    #[test]
    fn calculates_recommended_segments_adaptively() {
        assert_eq!(recommended_segment_count(0, 16), 4);
        assert_eq!(recommended_segment_count(500 * 1024, 16), 1);
        assert_eq!(recommended_segment_count(5 * 1024 * 1024, 16), 4);
        assert_eq!(recommended_segment_count(25 * 1024 * 1024, 16), 8);
        assert_eq!(recommended_segment_count(100 * 1024 * 1024, 16), 16);
        assert_eq!(recommended_segment_count(100 * 1024 * 1024, 4), 8);
        assert_eq!(recommended_segment_count(500 * 1024 * 1024, 32), 32);
    }
}
