use flate2::read::DeflateDecoder;
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

const SIG_LOCAL: u32 = 0x0403_4b50;
const SIG_CENTRAL: u32 = 0x0201_4b50;
const SIG_END: u32 = 0x0605_4b50;
const SIG_ZIP64_END: u32 = 0x0606_4b50;
const SIG_ZIP64_LOCATOR: u32 = 0x0706_4b50;

const END_LEN: usize = 22;
const LOCATOR_LEN: usize = 20;
const ZIP64_END_LEN: usize = 56;
const CENTRAL_LEN: usize = 46;
const LOCAL_LEN: usize = 30;
const TAIL_LEN: u64 = (END_LEN + 0xFFFF + LOCATOR_LEN + ZIP64_END_LEN) as u64;
const MAX_DIRECTORY: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteZipEntry {
    pub index: u32,
    pub path: String,
    pub size: u64,
    pub compressed_size: u64,
    pub method: u16,
    pub encrypted: bool,
    pub extractable: bool,
    pub header_offset: u64,
}

fn u16_le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_le(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

async fn fetch_range(
    client: &Client,
    url: &str,
    headers: Option<&HashMap<String, String>>,
    start: u64,
    end: u64,
) -> Result<Vec<u8>, String> {
    let req = client
        .get(url)
        .headers(crate::transfer::build_headers(headers)?)
        .header(header::RANGE, format!("bytes={start}-{end}"));
    let resp = req
        .send()
        .await
        .map_err(|e| format!("Range request failed: {}", e.without_url()))?;
    if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err("Server does not support HTTP Range requests for ZIP peek".into());
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    Ok(bytes.to_vec())
}

pub fn parse_central_directory(cd_bytes: &[u8], shift: u64) -> Result<Vec<RemoteZipEntry>, String> {
    let mut pos = 0usize;
    let mut out = Vec::new();
    let mut idx = 0u32;
    while pos + CENTRAL_LEN <= cd_bytes.len() {
        let sig = u32_le(cd_bytes, pos).ok_or("Truncated ZIP central directory")?;
        if sig != SIG_CENTRAL {
            break;
        }
        let flags = u16_le(cd_bytes, pos + 8).unwrap_or(0);
        let method = u16_le(cd_bytes, pos + 10).unwrap_or(0);
        let mut comp_size = u32_le(cd_bytes, pos + 20).unwrap_or(0) as u64;
        let mut uncomp_size = u32_le(cd_bytes, pos + 24).unwrap_or(0) as u64;
        let name_len = u16_le(cd_bytes, pos + 28).unwrap_or(0) as usize;
        let extra_len = u16_le(cd_bytes, pos + 30).unwrap_or(0) as usize;
        let comment_len = u16_le(cd_bytes, pos + 32).unwrap_or(0) as usize;
        let mut local_offset = u32_le(cd_bytes, pos + 42).unwrap_or(0) as u64;

        let name_start = pos + CENTRAL_LEN;
        let extra_start = name_start + name_len;
        let next = extra_start + extra_len + comment_len;
        if next > cd_bytes.len() {
            return Err("Truncated ZIP entry".into());
        }
        let name = String::from_utf8_lossy(&cd_bytes[name_start..extra_start]).into_owned();

        // Parse ZIP64 extended information extra field (0x0001) if needed.
        if comp_size == 0xFFFF_FFFF || uncomp_size == 0xFFFF_FFFF || local_offset == 0xFFFF_FFFF {
            let mut ep = extra_start;
            while ep + 4 <= extra_start + extra_len {
                let tag = u16_le(cd_bytes, ep).unwrap_or(0);
                let sz = u16_le(cd_bytes, ep + 2).unwrap_or(0) as usize;
                let data_start = ep + 4;
                if data_start + sz > extra_start + extra_len {
                    break;
                }
                if tag == 0x0001 {
                    let mut fp = data_start;
                    if uncomp_size == 0xFFFF_FFFF && fp + 8 <= data_start + sz {
                        uncomp_size = u64_le(cd_bytes, fp).unwrap_or(uncomp_size);
                        fp += 8;
                    }
                    if comp_size == 0xFFFF_FFFF && fp + 8 <= data_start + sz {
                        comp_size = u64_le(cd_bytes, fp).unwrap_or(comp_size);
                        fp += 8;
                    }
                    if local_offset == 0xFFFF_FFFF && fp + 8 <= data_start + sz {
                        local_offset = u64_le(cd_bytes, fp).unwrap_or(local_offset);
                    }
                }
                ep = data_start + sz;
            }
        }

        if !name.ends_with('/') {
            let encrypted = (flags & 1) != 0 || method == 99;
            let extractable = !encrypted && (method == 0 || method == 8);
            out.push(RemoteZipEntry {
                index: idx,
                path: name,
                size: uncomp_size,
                compressed_size: comp_size,
                method,
                encrypted,
                extractable,
                header_offset: local_offset.saturating_add(shift),
            });
            idx += 1;
        }
        pos = next;
    }
    Ok(out)
}

pub async fn list_remote_zip(
    client: &Client,
    url: &str,
    headers: Option<&HashMap<String, String>>,
) -> Result<Vec<RemoteZipEntry>, String> {
    let probe = crate::transfer::probe_resource(client, url, headers).await?;
    let total_size = probe
        .content_length
        .filter(|&n| n >= END_LEN as u64)
        .ok_or("Remote ZIP size is unknown or too small")?;
    if !probe.accept_ranges {
        return Err("Server does not support byte ranges".into());
    }

    let tail_start = total_size.saturating_sub(TAIL_LEN);
    let tail = fetch_range(client, url, headers, tail_start, total_size - 1).await?;
    if tail.len() < END_LEN {
        return Err("Not a valid ZIP archive".into());
    }

    // Locate End of Central Directory (search backwards).
    let eocd_rel = (0..=tail.len() - END_LEN)
        .rev()
        .find(|&i| {
            u32_le(&tail, i) == Some(SIG_END)
                && u16_le(&tail, i + 20)
                    .map(|c| i + END_LEN + c as usize == tail.len())
                    .unwrap_or(false)
        })
        .ok_or("End of Central Directory not found (not a ZIP file)")?;

    let disk_no = u16_le(&tail, eocd_rel + 4).unwrap_or(0);
    let cd_disk = u16_le(&tail, eocd_rel + 6).unwrap_or(0);
    if disk_no != 0 || cd_disk != 0 {
        return Err("Split ZIP archives are not supported".into());
    }

    let mut cd_size = u32_le(&tail, eocd_rel + 12).unwrap_or(0) as u64;
    let mut cd_offset = u32_le(&tail, eocd_rel + 16).unwrap_or(0) as u64;

    if (cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF) && eocd_rel >= LOCATOR_LEN {
        let loc_rel = eocd_rel - LOCATOR_LEN;
        if u32_le(&tail, loc_rel) == Some(SIG_ZIP64_LOCATOR) {
            let z64_off = u64_le(&tail, loc_rel + 8).unwrap_or(0);
            let z64_bytes = if z64_off >= tail_start
                && (z64_off - tail_start) as usize + ZIP64_END_LEN <= tail.len()
            {
                let rel = (z64_off - tail_start) as usize;
                tail[rel..rel + ZIP64_END_LEN].to_vec()
            } else {
                fetch_range(client, url, headers, z64_off, z64_off + ZIP64_END_LEN as u64 - 1)
                    .await?
            };
            if u32_le(&z64_bytes, 0) == Some(SIG_ZIP64_END) {
                cd_size = u64_le(&z64_bytes, 40).unwrap_or(cd_size);
                cd_offset = u64_le(&z64_bytes, 48).unwrap_or(cd_offset);
            }
        }
    }

    if cd_size > MAX_DIRECTORY || cd_offset.saturating_add(cd_size) > total_size {
        return Err("ZIP central directory is too large or out of bounds".into());
    }

    let cd_bytes = if cd_offset >= tail_start && cd_offset + cd_size <= total_size {
        let rel = (cd_offset - tail_start) as usize;
        tail[rel..rel + cd_size as usize].to_vec()
    } else {
        fetch_range(client, url, headers, cd_offset, cd_offset + cd_size - 1).await?
    };

    parse_central_directory(&cd_bytes, 0)
}

pub async fn extract_remote_zip_entry(
    client: &Client,
    url: &str,
    headers: Option<&HashMap<String, String>>,
    entry: &RemoteZipEntry,
    dest_path: &Path,
) -> Result<u64, String> {
    if !entry.extractable {
        return Err("Entry is encrypted or uses an unsupported compression method".into());
    }
    let local_hdr = fetch_range(
        client,
        url,
        headers,
        entry.header_offset,
        entry.header_offset + LOCAL_LEN as u64 - 1,
    )
    .await?;
    if local_hdr.len() < LOCAL_LEN || u32_le(&local_hdr, 0) != Some(SIG_LOCAL) {
        return Err("Invalid ZIP local file header".into());
    }
    let name_len = u16_le(&local_hdr, 26).unwrap_or(0) as u64;
    let extra_len = u16_le(&local_hdr, 28).unwrap_or(0) as u64;
    let data_start = entry.header_offset + LOCAL_LEN as u64 + name_len + extra_len;

    if entry.compressed_size == 0 {
        if let Some(parent) = dest_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
        }
        tokio::fs::write(dest_path, &[]).await.map_err(|e| e.to_string())?;
        return Ok(0);
    }

    let payload = fetch_range(
        client,
        url,
        headers,
        data_start,
        data_start + entry.compressed_size - 1,
    )
    .await?;

    let dest = dest_path.to_path_buf();
    let method = entry.method;
    tokio::task::spawn_blocking(move || -> Result<u64, String> {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out_file = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
        match method {
            0 => {
                use std::io::Write;
                out_file.write_all(&payload).map_err(|e| e.to_string())?;
                Ok(payload.len() as u64)
            }
            8 => {
                let mut decoder = DeflateDecoder::new(&payload[..]);
                let copied = std::io::copy(&mut decoder, &mut out_file).map_err(|e| e.to_string())?;
                Ok(copied)
            }
            _ => Err("Unsupported ZIP compression method".into()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    #[test]
    fn parses_zip_central_directory_from_memory() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut buf);
            zw.start_file("folder/hello.txt", SimpleFileOptions::default())
                .unwrap();
            zw.write_all(b"hello world from argon").unwrap();
            zw.finish().unwrap();
        }
        let bytes = buf.into_inner();
        let eocd_pos = (0..=bytes.len() - END_LEN)
            .rev()
            .find(|&i| u32_le(&bytes, i) == Some(SIG_END))
            .unwrap();
        let cd_size = u32_le(&bytes, eocd_pos + 12).unwrap() as usize;
        let cd_off = u32_le(&bytes, eocd_pos + 16).unwrap() as usize;
        let entries = parse_central_directory(&bytes[cd_off..cd_off + cd_size], 0).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "folder/hello.txt");
        assert_eq!(entries[0].size, 22);
        assert!(entries[0].extractable);
    }
}
