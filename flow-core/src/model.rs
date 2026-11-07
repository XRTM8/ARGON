use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub const PROTOCOL: &str = "argon-flow/2";
pub const DISCOVERY_PORT: u16 = 54126;
pub const TRANSFER_PORT: u16 = 54127;
pub const MAX_FILE: u64 = 50 * 1024 * 1024 * 1024;
pub const MAX_TEXT: usize = 16 * 1024;

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn uid() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
pub fn safe_name(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(40)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub address: String,
    pub trusted: bool,
    pub online: bool,
    pub last_seen: u64,
}
impl Peer {
    pub fn endpoint(&self) -> Result<SocketAddr, String> {
        self.address
            .parse()
            .map_err(|_| "Invalid device address".into())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedPeer {
    pub id: String,
    pub name: String,
    pub address: String,
    pub paired_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub id: String,
    pub peer_id: String,
    pub peer_name: String,
    pub name: String,
    pub direction: String,
    pub kind: String,
    pub status: String,
    pub bytes: u64,
    pub total: u64,
    pub speed: f64,
    pub created_at: u64,
    pub path: Option<String>,
    pub error: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pairing {
    pub id: String,
    pub peer_id: String,
    pub peer_name: String,
    pub code: String,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Incoming {
    pub id: String,
    pub peer_id: String,
    pub peer_name: String,
    pub name: String,
    pub size: u64,
    pub kind: String,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardItem {
    pub id: String,
    pub peer_name: String,
    pub text: String,
    pub received_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedFile {
    pub id: String,
    pub name: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub enabled: bool,
    pub id: String,
    pub name: String,
    pub port: u16,
    pub addresses: Vec<String>,
    pub receive_directory: String,
    pub peers: Vec<Peer>,
    pub transfers: Vec<Transfer>,
    pub pairings: Vec<Pairing>,
    pub incoming: Vec<Incoming>,
    pub clipboard: Vec<ClipboardItem>,
    pub last_error: Option<String>,
    pub protocol: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "kebab-case")]
pub enum Event {
    Changed,
    Pairing(Pairing),
    Incoming(Incoming),
    Error(String),
}

pub struct Config {
    pub storage: PathBuf,
    pub receive_directory: PathBuf,
    pub name: String,
    pub bind_address: std::net::IpAddr,
    pub port: u16,
    pub discovery: bool,
}
impl Config {
    pub fn desktop(storage: PathBuf, receive_directory: PathBuf, name: String) -> Self {
        Self {
            storage,
            receive_directory,
            name,
            bind_address: std::net::Ipv4Addr::UNSPECIFIED.into(),
            port: TRANSFER_PORT,
            discovery: true,
        }
    }
}
