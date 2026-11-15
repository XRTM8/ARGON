use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebridUnrestrictResult {
    pub id: String,
    pub filename: String,
    pub download_url: String,
    pub filesize: Option<u64>,
    pub original_link: String,
}

pub fn is_debrid_supported_host(raw_url: &str) -> bool {
    let lower = raw_url.trim().to_ascii_lowercase();
    if lower.starts_with("magnet:") {
        return true;
    }
    let Ok(url) = reqwest::Url::parse(&lower) else {
        return false;
    };
    let host = url.host_str().unwrap_or("");
    const SUPPORTED_HOSTS: &[&str] = &[
        "rapidgator.net",
        "rg.to",
        "1fichier.com",
        "turbobit.net",
        "ddownload.com",
        "katfile.com",
        "nitroflare.com",
        "filefactory.com",
        "mega.nz",
        "alfafile.net",
        "dropgalaxy.com",
        "uptobox.com",
        "upstore.net",
        "mediafire.com",
        "mexashare.com",
        "rosefile.net",
        "fikper.com",
    ];
    SUPPORTED_HOSTS
        .iter()
        .any(|&domain| host == domain || host.ends_with(&format!(".{domain}")))
}

pub async fn unrestrict_link(
    client: &Client,
    api_key: &str,
    link: &str,
    password: Option<&str>,
) -> Result<DebridUnrestrictResult, String> {
    let api_url = "https://api.real-debrid.com/rest/1.0/unrestrict/link";
    let mut form = vec![("link", link)];
    if let Some(pwd) = password {
        form.push(("password", pwd));
    }

    let resp = tokio::time::timeout(
        Duration::from_secs(20),
        client
            .post(api_url)
            .header("Authorization", format!("Bearer {}", api_key.trim()))
            .form(&form)
            .send(),
    )
    .await
    .map_err(|_| "انتهت مهلة الاتصال بخدمة Real-Debrid".to_string())?
    .map_err(|e| format!("فشل الاتصال بخدمة Real-Debrid: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let err_body = resp.text().await.unwrap_or_default();
        return Err(format!("خطأ Real-Debrid (HTTP {status}): {err_body}"));
    }

    #[derive(Deserialize)]
    struct RdResponse {
        id: String,
        filename: String,
        download: String,
        filesize: Option<u64>,
        link: String,
    }

    let parsed: RdResponse = resp
        .json()
        .await
        .map_err(|e| format!("استجابة غير متوقعة من Real-Debrid: {e}"))?;

    Ok(DebridUnrestrictResult {
        id: parsed.id,
        filename: parsed.filename,
        download_url: parsed.download,
        filesize: parsed.filesize,
        original_link: parsed.link,
    })
}

pub async fn unrestrict_magnet(
    client: &Client,
    api_key: &str,
    magnet_uri: &str,
) -> Result<DebridUnrestrictResult, String> {
    let add_url = "https://api.real-debrid.com/rest/1.0/torrents/addMagnet";
    let add_resp = tokio::time::timeout(
        Duration::from_secs(20),
        client
            .post(add_url)
            .header("Authorization", format!("Bearer {}", api_key.trim()))
            .form(&[("magnet", magnet_uri)])
            .send(),
    )
    .await
    .map_err(|_| "انتهت مهلة إضافة التورنت إلى Real-Debrid".to_string())?
    .map_err(|e| format!("فشل إضافة التورنت إلى Real-Debrid: {e}"))?;

    if !add_resp.status().is_success() {
        let err = add_resp.text().await.unwrap_or_default();
        return Err(format!("فشل إضافة الماغنت إلى Real-Debrid: {err}"));
    }

    #[derive(Deserialize)]
    struct AddTorrentResp {
        id: String,
    }
    let add_data: AddTorrentResp = add_resp
        .json()
        .await
        .map_err(|e| format!("استجابة غير متوقعة من Real-Debrid: {e}"))?;

    // Select all files in torrent
    let select_url = format!(
        "https://api.real-debrid.com/rest/1.0/torrents/selectFiles/{}",
        add_data.id
    );
    let _ = client
        .post(&select_url)
        .header("Authorization", format!("Bearer {}", api_key.trim()))
        .form(&[("files", "all")])
        .send()
        .await;

    // Poll torrent status (up to 5 attempts with 600ms backoff) for cached torrents
    let info_url = format!(
        "https://api.real-debrid.com/rest/1.0/torrents/info/{}",
        add_data.id
    );

    #[derive(Deserialize)]
    struct TorrentInfoResp {
        status: Option<String>,
        links: Option<Vec<String>>,
    }

    let mut links = Vec::new();
    let mut last_status = "unknown".to_string();

    for attempt in 0..5 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(600)).await;
        }

        let info_resp = client
            .get(&info_url)
            .header("Authorization", format!("Bearer {}", api_key.trim()))
            .send()
            .await;

        if let Ok(resp) = info_resp {
            if let Ok(info_data) = resp.json::<TorrentInfoResp>().await {
                if let Some(s) = info_data.status {
                    last_status = s;
                }
                if let Some(l) = info_data.links {
                    if !l.is_empty() {
                        links = l;
                        break;
                    }
                }
            }
        }
    }

    if links.is_empty() {
        return Err(format!(
            "التورنت قيد التحويل أو التحميل السحابي على Real-Debrid (الحالة: {last_status}). أعد المحاولة بعد قليل."
        ));
    }

    // Unrestrict first direct link
    unrestrict_link(client, api_key, &links[0], None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_supported_debrid_hosts() {
        assert!(is_debrid_supported_host("https://rapidgator.net/file/12345/archive.part1.rar.html"));
        assert!(is_debrid_supported_host("https://rg.to/file/abc/file.zip"));
        assert!(is_debrid_supported_host("https://1fichier.com/?abcdef"));
        assert!(is_debrid_supported_host("https://mega.nz/file/xyz#123"));
        assert!(is_debrid_supported_host("https://turbobit.net/abc.html"));
        assert!(is_debrid_supported_host("magnet:?xt=urn:btih:0123456789abcdef"));
        assert!(!is_debrid_supported_host("https://example.com/file.zip"));
        assert!(!is_debrid_supported_host("https://github.com/torvalds/linux"));
    }
}
