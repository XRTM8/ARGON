use reqwest::Client;
use serde::{Deserialize, Serialize};

pub const DEFAULT_HF_ENDPOINT: &str = "https://huggingface.co";
pub const MIRROR_HF_ENDPOINT: &str = "https://hf-mirror.com";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HfFileEntry {
    pub path: String,
    pub size: u64,
    pub sha256: Option<String>,
    pub download_url: String,
    pub quant: Option<String>,
    pub is_config: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HfRepoSummary {
    pub repo_id: String,
    pub kind: String,
    pub endpoint: String,
    pub files: Vec<HfFileEntry>,
    pub quants: Vec<String>,
}

pub fn normalize_hf_repo(input: &str) -> Option<(String, String)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(url) = reqwest::Url::parse(trimmed) {
        let host = url.host_str()?.to_ascii_lowercase();
        if !host.ends_with("huggingface.co") && !host.ends_with("hf-mirror.com") {
            return None;
        }
        let segs: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
        if segs.first() == Some(&"datasets") && segs.len() >= 3 {
            return Some(("datasets".into(), format!("{}/{}", segs[1], segs[2])));
        }
        if segs.len() >= 2 {
            return Some(("models".into(), format!("{}/{}", segs[0], segs[1])));
        }
        return None;
    }
    let parts: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() == 2 {
        return Some(("models".into(), format!("{}/{}", parts[0], parts[1])));
    }
    None
}

fn detect_gguf_quant(path: &str) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    if !lower.ends_with(".gguf") {
        return None;
    }
    let fname = path.rsplit('/').next().unwrap_or(path);
    let stem = fname.strip_suffix(".gguf").unwrap_or(fname);
    for token in stem.rsplit(['-', '.']) {
        let up = token.to_ascii_uppercase();
        if (up.starts_with('Q') || up.starts_with("IQ") || up == "F16" || up == "BF16" || up == "F32")
            && up.len() <= 10
        {
            return Some(up);
        }
    }
    Some("GGUF".into())
}

pub async fn fetch_hf_repo(
    client: &Client,
    input: &str,
    use_mirror: bool,
    token: Option<&str>,
) -> Result<HfRepoSummary, String> {
    let (kind, repo_id) =
        normalize_hf_repo(input).ok_or("Enter a valid Hugging Face repo like Qwen/Qwen3-8B or URL")?;
    let endpoint = if use_mirror {
        MIRROR_HF_ENDPOINT
    } else {
        DEFAULT_HF_ENDPOINT
    };
    let api_url = format!("{endpoint}/api/{kind}/{repo_id}/tree/main?recursive=1");
    let mut req = client.get(&api_url);
    if let Some(tok) = token.map(str::trim).filter(|s| !s.is_empty()) {
        req = req.bearer_auth(tok);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("Hugging Face request failed: {}", e.without_url()))?;
    if !resp.status().is_success() {
        return Err(format!("Hugging Face API returned HTTP {}", resp.status()));
    }
    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let arr = json.as_array().ok_or("Unexpected Hugging Face API response")?;

    let url_prefix = if kind == "datasets" {
        format!("{endpoint}/datasets/{repo_id}/resolve/main")
    } else {
        format!("{endpoint}/{repo_id}/resolve/main")
    };

    let mut files = Vec::new();
    let mut quants = Vec::new();
    for item in arr {
        if item.get("type").and_then(|v| v.as_str()) != Some("file") {
            continue;
        }
        let Some(path) = item.get("path").and_then(|v| v.as_str()) else {
            continue;
        };
        let size = item
            .get("lfs")
            .and_then(|l| l.get("size"))
            .and_then(|v| v.as_u64())
            .or_else(|| item.get("size").and_then(|v| v.as_u64()))
            .unwrap_or(0);
        let sha256 = item
            .get("lfs")
            .and_then(|l| l.get("oid"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let quant = detect_gguf_quant(path);
        if let Some(ref q) = quant {
            if !quants.contains(q) {
                quants.push(q.clone());
            }
        }
        let is_config = size <= 16 * 1024 * 1024
            && (path.ends_with(".json")
                || path.ends_with(".md")
                || path.ends_with(".txt")
                || path.ends_with(".model"));
        files.push(HfFileEntry {
            path: path.to_string(),
            size,
            sha256,
            download_url: format!("{url_prefix}/{path}"),
            quant,
            is_config,
        });
    }

    Ok(HfRepoSummary {
        repo_id,
        kind,
        endpoint: endpoint.to_string(),
        files,
        quants,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_hf_inputs() {
        assert_eq!(
            normalize_hf_repo("Qwen/Qwen3-8B"),
            Some(("models".into(), "Qwen/Qwen3-8B".into()))
        );
        assert_eq!(
            normalize_hf_repo("https://huggingface.co/datasets/wikimedia/wikipedia/tree/main"),
            Some(("datasets".into(), "wikimedia/wikipedia".into()))
        );
    }

    #[test]
    fn detects_gguf_quant_tags() {
        assert_eq!(
            detect_gguf_quant("model-Q4_K_M.gguf"),
            Some("Q4_K_M".into())
        );
        assert_eq!(
            detect_gguf_quant("Qwen3-8B-Q8_0.gguf"),
            Some("Q8_0".into())
        );
    }
}
