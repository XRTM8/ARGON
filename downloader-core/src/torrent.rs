use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TorrentFileItem {
    pub path: String,
    pub length: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TorrentMetadata {
    pub info_hash: String,
    pub name: String,
    pub total_length: u64,
    pub piece_length: Option<u64>,
    pub piece_count: Option<usize>,
    pub trackers: Vec<String>,
    pub files: Vec<TorrentFileItem>,
    pub comment: Option<String>,
    pub is_magnet: bool,
}

/// Standard RFC 3174 compliant SHA-1 implementation
#[allow(clippy::chunks_exact_to_as_chunks)]
pub fn sha1_digest(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let total_len = data.len();
    let mut padded = Vec::with_capacity(total_len + 72);
    padded.extend_from_slice(data);
    padded.push(0x80);

    while (padded.len() % 64) != 56 {
        padded.push(0x00);
    }

    let bit_len = (total_len as u64).wrapping_mul(8);
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, item) in w.iter_mut().take(16).enumerate() {
            let offset = i * 4;
            *item = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;

        for (i, &item) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };

            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(item);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

pub fn sha1_hex(data: &[u8]) -> String {
    let digest = sha1_digest(data);
    let mut s = String::with_capacity(40);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// Validates whether a string is a BitTorrent Magnet URI.
pub fn is_magnet_uri(raw: &str) -> bool {
    let trimmed = raw.trim();
    if !trimmed.starts_with("magnet:?") {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.contains("xt=urn:btih:")
}

/// Decodes RFC 4648 Base32 string to 40-character lowercase hexadecimal.
pub fn base32_to_hex(base32_str: &str) -> Result<String, String> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let clean = base32_str.trim().trim_end_matches('=').to_ascii_uppercase();
    let mut bits = String::new();

    for c in clean.chars() {
        let b = c as u8;
        let idx = ALPHABET
            .iter()
            .position(|&a| a == b)
            .ok_or_else(|| format!("Invalid Base32 character: {c}"))?;
        use std::fmt::Write;
        let _ = write!(bits, "{:05b}", idx);
    }

    let mut hex = String::with_capacity(40);
    let bit_bytes = bits.as_bytes();
    let mut i = 0;
    while i + 4 <= bit_bytes.len() && hex.len() < 40 {
        let chunk_str = std::str::from_utf8(&bit_bytes[i..i + 4]).unwrap_or("0000");
        let val = u8::from_str_radix(chunk_str, 2).unwrap_or(0);
        use std::fmt::Write;
        let _ = write!(hex, "{:x}", val);
        i += 4;
    }

    if hex.len() < 40 {
        return Err("Base32 data too short for 20-byte SHA-1 hash".to_string());
    }

    Ok(hex[..40].to_ascii_lowercase())
}

/// Parses a BitTorrent Magnet link into structured TorrentMetadata.
pub fn parse_magnet_uri(raw_uri: &str) -> Result<TorrentMetadata, String> {
    if !is_magnet_uri(raw_uri) {
        return Err("Not a valid BitTorrent Magnet URI".to_string());
    }

    let query_str = raw_uri.trim().strip_prefix("magnet:?").unwrap_or("");
    let mut info_hash = String::new();
    let mut name = String::new();
    let mut trackers = Vec::new();
    let mut total_length: u64 = 0;

    for param in query_str.split('&') {
        if param.is_empty() {
            continue;
        }
        let (key, val) = match param.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };

        let decoded_val = urlencoding_decode(val);

        if key.eq_ignore_ascii_case("xt") {
            let lower = decoded_val.to_ascii_lowercase();
            if let Some(hash_part) = lower.strip_prefix("urn:btih:") {
                let clean_hash = hash_part.trim();
                if clean_hash.len() == 40 && clean_hash.chars().all(|c| c.is_ascii_hexdigit()) {
                    info_hash = clean_hash.to_string();
                } else if clean_hash.len() == 32 {
                    if let Ok(hex) = base32_to_hex(clean_hash) {
                        info_hash = hex;
                    }
                }
            }
        } else if key.eq_ignore_ascii_case("dn") {
            name = decoded_val;
        } else if key.eq_ignore_ascii_case("tr") {
            let clean_tr = decoded_val.trim().to_string();
            if !clean_tr.is_empty() && !trackers.contains(&clean_tr) {
                trackers.push(clean_tr);
            }
        } else if key.eq_ignore_ascii_case("xl") {
            if let Ok(len) = decoded_val.parse::<u64>() {
                total_length = len;
            }
        }
    }

    if info_hash.is_empty() {
        return Err("Magnet URI is missing a valid urn:btih hash".to_string());
    }

    if name.is_empty() {
        name = format!("Torrent_{}", &info_hash[..8]);
    }

    let files = if total_length > 0 {
        vec![TorrentFileItem {
            path: name.clone(),
            length: total_length,
        }]
    } else {
        Vec::new()
    };

    Ok(TorrentMetadata {
        info_hash,
        name,
        total_length,
        piece_length: None,
        piece_count: None,
        trackers,
        files,
        comment: None,
        is_magnet: true,
    })
}

fn urlencoding_decode(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let mut chars = input.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h1 = chars.next().unwrap_or(b'0');
            let h2 = chars.next().unwrap_or(b'0');
            let hex_str = [h1, h2];
            if let Ok(s) = std::str::from_utf8(&hex_str) {
                if let Ok(byte) = u8::from_str_radix(s, 16) {
                    bytes.push(byte);
                    continue;
                }
            }
            bytes.push(b'%');
            bytes.push(h1);
            bytes.push(h2);
        } else if b == b'+' {
            bytes.push(b' ');
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8_lossy(&bytes).to_string()
}

/// Represents a parsed Bencode data value (BEP 0003)
#[derive(Clone, Debug, PartialEq)]
pub enum BencodeValue {
    Integer(i64),
    Bytes(Vec<u8>),
    List(Vec<BencodeValue>),
    Dict(BTreeMap<String, BencodeValue>),
}

impl BencodeValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            BencodeValue::Bytes(b) => std::str::from_utf8(b).ok(),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            BencodeValue::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[BencodeValue]> {
        match self {
            BencodeValue::List(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&BTreeMap<String, BencodeValue>> {
        match self {
            BencodeValue::Dict(d) => Some(d),
            _ => None,
        }
    }
}

pub struct BencodeParser<'a> {
    data: &'a [u8],
    pos: usize,
    depth: usize,
    pub raw_info_slice: Option<&'a [u8]>,
}

impl<'a> BencodeParser<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            depth: 0,
            raw_info_slice: None,
        }
    }

    pub fn parse(&mut self) -> Result<BencodeValue, String> {
        self.pos = 0;
        self.depth = 0;
        self.raw_info_slice = None;
        self.parse_next()
    }

    fn parse_next(&mut self) -> Result<BencodeValue, String> {
        if self.pos >= self.data.len() {
            return Err("Unexpected end of Bencode input".to_string());
        }
        if self.depth > 64 {
            return Err("Bencode recursion depth limit exceeded".to_string());
        }

        let byte = self.data[self.pos];
        match byte {
            b'i' => self.parse_integer(),
            b'l' => self.parse_list(),
            b'd' => self.parse_dictionary(),
            b'0'..=b'9' => self.parse_bytes(),
            _ => Err(format!(
                "Invalid Bencode token 0x{:02x} at position {}",
                byte, self.pos
            )),
        }
    }

    fn parse_integer(&mut self) -> Result<BencodeValue, String> {
        self.pos += 1; // skip 'i'
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != b'e' {
            self.pos += 1;
        }
        if self.pos >= self.data.len() {
            return Err("Unterminated Bencode integer".to_string());
        }
        let raw = &self.data[start..self.pos];
        self.pos += 1; // skip 'e'
        let s = std::str::from_utf8(raw).map_err(|e| e.to_string())?;
        let val = s.parse::<i64>().map_err(|e| e.to_string())?;
        Ok(BencodeValue::Integer(val))
    }

    fn parse_bytes(&mut self) -> Result<BencodeValue, String> {
        if self.pos >= self.data.len() || !self.data[self.pos].is_ascii_digit() {
            return Err(format!(
                "Expected ASCII digit at position {}, found {:?}",
                self.pos,
                self.data.get(self.pos).map(|&b| b as char)
            ));
        }
        let colon = self.data[self.pos..]
            .iter()
            .position(|&b| b == b':')
            .ok_or_else(|| "Missing colon in Bencode string prefix".to_string())?;
        let len_str = std::str::from_utf8(&self.data[self.pos..self.pos + colon])
            .map_err(|e| e.to_string())?;
        let len = len_str.parse::<usize>().map_err(|e| e.to_string())?;

        self.pos += colon + 1;
        if self.pos + len > self.data.len() {
            return Err("Bencode byte string out of bounds".to_string());
        }
        let slice = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(BencodeValue::Bytes(slice.to_vec()))
    }

    fn parse_list(&mut self) -> Result<BencodeValue, String> {
        self.pos += 1; // skip 'l'
        self.depth += 1;
        let mut list = Vec::new();
        while self.pos < self.data.len() && self.data[self.pos] != b'e' {
            list.push(self.parse_next()?);
        }
        if self.pos >= self.data.len() {
            return Err("Unterminated Bencode list".to_string());
        }
        self.pos += 1; // skip 'e'
        self.depth -= 1;
        Ok(BencodeValue::List(list))
    }

    fn parse_dictionary(&mut self) -> Result<BencodeValue, String> {
        self.pos += 1; // skip 'd'
        self.depth += 1;
        let mut map = BTreeMap::new();

        while self.pos < self.data.len() && self.data[self.pos] != b'e' {
            let key_val = self.parse_bytes()?;
            let key = match key_val {
                BencodeValue::Bytes(b) => String::from_utf8_lossy(&b).to_string(),
                _ => return Err("Expected byte string key in Bencode dictionary".to_string()),
            };

            let val_start = self.pos;
            let val = self.parse_next()?;
            let val_end = self.pos;

            if key == "info" {
                self.raw_info_slice = Some(&self.data[val_start..val_end]);
            }

            map.insert(key, val);
        }

        if self.pos >= self.data.len() {
            return Err("Unterminated Bencode dictionary".to_string());
        }
        self.pos += 1; // skip 'e'
        self.depth -= 1;
        Ok(BencodeValue::Dict(map))
    }
}

/// Parses raw `.torrent` file bytes into comprehensive TorrentMetadata.
pub fn parse_torrent_bytes(bytes: &[u8]) -> Result<TorrentMetadata, String> {
    let mut parser = BencodeParser::new(bytes);
    let root = parser.parse()?;

    let dict = match root {
        BencodeValue::Dict(d) => d,
        _ => return Err("Invalid .torrent: root is not a Bencode dictionary".to_string()),
    };

    let raw_info = parser
        .raw_info_slice
        .ok_or_else(|| "Invalid .torrent: missing 'info' dictionary".to_string())?;

    let info_hash = sha1_hex(raw_info);

    let info = dict
        .get("info")
        .and_then(|v| v.as_dict())
        .ok_or_else(|| "Invalid .torrent: 'info' must be a dictionary".to_string())?;

    let name = info
        .get("name.utf-8")
        .or_else(|| info.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("Torrent_{}", &info_hash[..8]));

    let mut trackers = Vec::new();
    if let Some(ann) = dict.get("announce").and_then(|v| v.as_str()) {
        let t = ann.trim().to_string();
        if !t.is_empty() {
            trackers.push(t);
        }
    }
    if let Some(list) = dict.get("announce-list").and_then(|v| v.as_list()) {
        for tier in list {
            if let Some(tier_list) = tier.as_list() {
                for tr in tier_list {
                    if let Some(s) = tr.as_str() {
                        let t = s.trim().to_string();
                        if !t.is_empty() && !trackers.contains(&t) {
                            trackers.push(t);
                        }
                    }
                }
            }
        }
    }

    let mut files = Vec::new();
    let mut total_length: u64 = 0;

    if let Some(len) = info.get("length").and_then(|v| v.as_i64()) {
        let ulen = len.max(0) as u64;
        total_length = ulen;
        files.push(TorrentFileItem {
            path: sanitize_torrent_filename(&name),
            length: ulen,
        });
    } else if let Some(files_list) = info.get("files").and_then(|v| v.as_list()) {
        for f in files_list {
            if let Some(f_dict) = f.as_dict() {
                let flen = f_dict
                    .get("length")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
                    .max(0) as u64;
                total_length += flen;

                let path_segments = f_dict
                    .get("path.utf-8")
                    .or_else(|| f_dict.get("path"))
                    .and_then(|v| v.as_list());

                let mut clean_segments = Vec::new();
                if let Some(segs) = path_segments {
                    for seg in segs {
                        if let Some(s) = seg.as_str() {
                            let clean = sanitize_torrent_filename(s);
                            if !clean.is_empty() && clean != "." && clean != ".." {
                                clean_segments.push(clean);
                            }
                        }
                    }
                }

                let sub_path = clean_segments.join("/");
                let safe_root = sanitize_torrent_filename(&name);
                let full_rel_path = if sub_path.is_empty() {
                    safe_root
                } else {
                    format!("{safe_root}/{sub_path}")
                };

                files.push(TorrentFileItem {
                    path: full_rel_path,
                    length: flen,
                });
            }
        }
    }

    let piece_length = info
        .get("piece length")
        .and_then(|v| v.as_i64())
        .map(|l| l.max(0) as u64);

    let piece_count = info.get("pieces").and_then(|v| match v {
        BencodeValue::Bytes(b) => Some(b.len() / 20),
        _ => None,
    });

    let comment = dict
        .get("comment.utf-8")
        .or_else(|| dict.get("comment"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(TorrentMetadata {
        info_hash,
        name,
        total_length,
        piece_length,
        piece_count,
        trackers,
        files,
        comment,
        is_magnet: false,
    })
}

fn sanitize_torrent_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'..='\x1f') {
            out.push('_');
        } else {
            out.push(c);
        }
    }
    let trimmed = out.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "torrent_file".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha1_standard_vectors() {
        assert_eq!(
            sha1_hex(b""),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
        assert_eq!(
            sha1_hex(b"The quick brown fox jumps over the lazy dog"),
            "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12"
        );
        assert_eq!(
            sha1_hex(b"The quick brown fox jumps over the lazy cog"),
            "de9f2c7fd25e1b3afad3e85a0bd17d9b100db4b3"
        );
    }

    #[test]
    fn test_parse_magnet_uri_hex() {
        let uri = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Ubuntu+24.04&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce&xl=1048576";
        let meta = parse_magnet_uri(uri).expect("Should parse magnet");
        assert_eq!(meta.info_hash, "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(meta.name, "Ubuntu 24.04");
        assert_eq!(meta.total_length, 1048576);
        assert_eq!(meta.trackers, vec!["udp://tracker.opentrackr.org:1337/announce"]);
        assert!(meta.is_magnet);
        assert_eq!(meta.files.len(), 1);
    }

    #[test]
    fn test_parse_magnet_uri_base32() {
        // Base32 for 20 bytes of zeros is AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA (32 A's)
        let uri = "magnet:?xt=urn:btih:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&dn=ZeroTorrent";
        let meta = parse_magnet_uri(uri).expect("Should parse Base32 magnet");
        assert_eq!(meta.info_hash, "0000000000000000000000000000000000000000");
        assert_eq!(meta.name, "ZeroTorrent");
    }

    #[test]
    fn test_bencode_and_torrent_bytes() {
        // Valid single-file torrent payload: announce string is 15 bytes ("http://tr.com/a")
        let torrent_data = b"d8:announce15:http://tr.com/a4:infod6:lengthi524288e4:name10:sample.bin12:piece lengthi16384e6:pieces20:12345678901234567890ee";
        let meta = parse_torrent_bytes(torrent_data).expect("Should parse torrent bytes");
        assert_eq!(meta.name, "sample.bin");
        assert_eq!(meta.total_length, 524288);
        assert_eq!(meta.piece_length, Some(16384));
        assert_eq!(meta.piece_count, Some(1));
        assert_eq!(meta.trackers, vec!["http://tr.com/a"]);
        assert_eq!(meta.files.len(), 1);
        assert_eq!(meta.files[0].path, "sample.bin");
        assert_eq!(meta.files[0].length, 524288);
        assert!(!meta.is_magnet);
    }
}
