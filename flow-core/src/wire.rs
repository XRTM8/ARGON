use crate::{
    identity::{fingerprint, Identity, NOISE},
    model::PROTOCOL,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

const MAGIC: &[u8] = b"ARGONFLOW2\n";
pub const DATA_SIZE: usize = 48 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Hello {
        name: String,
        id: String,
        protocol: String,
        port: u16,
        paired: bool,
    },
    Decision {
        accepted: bool,
    },
    Ping,
    Pong,
    File {
        id: String,
        name: String,
        size: u64,
        sha256: String,
    },
    Ready {
        offset: u64,
        prefix_sha256: String,
    },
    Resume {
        reset: bool,
    },
    End {
        sha256: String,
    },
    Complete {
        size: u64,
        sha256: String,
    },
    Text {
        id: String,
        text: String,
    },
    Error {
        message: String,
    },
}

pub struct Channel {
    stream: TcpStream,
    transport: snow::TransportState,
    pub peer_id: String,
    pub code: String,
}
impl Channel {
    pub async fn handshake(
        mut stream: TcpStream,
        identity: &Identity,
        initiator: bool,
    ) -> Result<Self, String> {
        stream.set_nodelay(true).map_err(|e| e.to_string())?;
        let builder = snow::Builder::new(NOISE.parse().map_err(|e| format!("{e}"))?)
            .local_private_key(&identity.private)
            .map_err(|e| e.to_string())?
            .prologue(PROTOCOL.as_bytes())
            .map_err(|e| e.to_string())?;
        let mut state = if initiator {
            builder.build_initiator()
        } else {
            builder.build_responder()
        }
        .map_err(|e| e.to_string())?;
        let mut buffer = vec![0u8; 65535];
        if initiator {
            stream.write_all(MAGIC).await.map_err(|e| e.to_string())?;
            let n = state
                .write_message(&[], &mut buffer)
                .map_err(|e| e.to_string())?;
            write_frame(&mut stream, &buffer[..n]).await?;
            let frame = read_frame(&mut stream).await?;
            state
                .read_message(&frame, &mut buffer)
                .map_err(|e| format!("Secure handshake failed: {e}"))?;
            let n = state
                .write_message(&[], &mut buffer)
                .map_err(|e| e.to_string())?;
            write_frame(&mut stream, &buffer[..n]).await?;
        } else {
            let mut magic = vec![0; MAGIC.len()];
            stream
                .read_exact(&mut magic)
                .await
                .map_err(|e| e.to_string())?;
            if magic != MAGIC {
                return Err(
                    "This peer uses an incompatible Flow protocol; update both devices".into(),
                );
            }
            let frame = read_frame(&mut stream).await?;
            state
                .read_message(&frame, &mut buffer)
                .map_err(|e| e.to_string())?;
            let n = state
                .write_message(&[], &mut buffer)
                .map_err(|e| e.to_string())?;
            write_frame(&mut stream, &buffer[..n]).await?;
            let frame = read_frame(&mut stream).await?;
            state
                .read_message(&frame, &mut buffer)
                .map_err(|e| format!("Secure handshake failed: {e}"))?;
        }
        let peer_id = fingerprint(state.get_remote_static().ok_or("Missing peer identity")?);
        let hash = state.get_handshake_hash();
        let code = format!(
            "{:06}",
            u32::from_be_bytes(hash[..4].try_into().map_err(|_| "Invalid handshake hash")?)
                % 1_000_000
        );
        let transport = state.into_transport_mode().map_err(|e| e.to_string())?;
        Ok(Self {
            stream,
            transport,
            peer_id,
            code,
        })
    }
    pub async fn send(&mut self, message: &Message) -> Result<(), String> {
        let body = serde_json::to_vec(message).map_err(|e| e.to_string())?;
        if body.len() > 65500 {
            return Err("Control message exceeds the allowed size".into());
        }
        let mut record = Vec::with_capacity(body.len() + 1);
        record.push(0);
        record.extend_from_slice(&body);
        self.send_record(&record).await
    }
    pub async fn receive(&mut self) -> Result<Message, String> {
        let record = self.receive_record().await?;
        if record.first() != Some(&0) {
            return Err("Unexpected data message".into());
        }
        let message: Message = serde_json::from_slice(&record[1..]).map_err(|e| e.to_string())?;
        if let Message::Error { message } = message {
            return Err(message);
        }
        Ok(message)
    }
    pub async fn send_data(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > DATA_SIZE {
            return Err("Data chunk too large".into());
        }
        let mut record = Vec::with_capacity(bytes.len() + 1);
        record.push(1);
        record.extend_from_slice(bytes);
        self.send_record(&record).await
    }
    async fn send_record(&mut self, plain: &[u8]) -> Result<(), String> {
        let mut encrypted = vec![0; plain.len() + 16];
        let n = self
            .transport
            .write_message(plain, &mut encrypted)
            .map_err(|e| format!("Encryption failed: {e}"))?;
        tokio::time::timeout(
            Duration::from_secs(30),
            write_frame(&mut self.stream, &encrypted[..n]),
        )
        .await
        .map_err(|_| "Send timed out".to_owned())?
    }
    pub async fn receive_record(&mut self) -> Result<Vec<u8>, String> {
        let encrypted =
            tokio::time::timeout(Duration::from_secs(120), read_frame(&mut self.stream))
                .await
                .map_err(|_| "Peer stopped responding".to_owned())??;
        let mut plain = vec![0; encrypted.len()];
        let n = self
            .transport
            .read_message(&encrypted, &mut plain)
            .map_err(|_| "Authentication failed: tampered or replayed data".to_owned())?;
        plain.truncate(n);
        if plain.is_empty() {
            return Err("Empty encrypted record".into());
        }
        Ok(plain)
    }
}
async fn write_frame(stream: &mut TcpStream, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 16 || bytes.len() > 65535 {
        return Err("Invalid frame size".into());
    }
    stream
        .write_u16(bytes.len() as u16)
        .await
        .map_err(|e| e.to_string())?;
    stream.write_all(bytes).await.map_err(|e| e.to_string())
}
async fn read_frame(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    let size = stream.read_u16().await.map_err(|e| e.to_string())? as usize;
    if size < 16 {
        return Err("Invalid frame size".into());
    }
    let mut bytes = vec![0; size];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn transports() -> (snow::TransportState, snow::TransportState) {
        let params: snow::params::NoiseParams = NOISE.parse().unwrap();
        let a = snow::Builder::new(params.clone())
            .generate_keypair()
            .unwrap();
        let b = snow::Builder::new(params.clone())
            .generate_keypair()
            .unwrap();
        let mut a = snow::Builder::new(params.clone())
            .local_private_key(&a.private)
            .unwrap()
            .prologue(PROTOCOL.as_bytes())
            .unwrap()
            .build_initiator()
            .unwrap();
        let mut b = snow::Builder::new(params)
            .local_private_key(&b.private)
            .unwrap()
            .prologue(PROTOCOL.as_bytes())
            .unwrap()
            .build_responder()
            .unwrap();
        let mut message = [0u8; 1024];
        let mut plain = [0u8; 1024];
        let size = a.write_message(&[], &mut message).unwrap();
        b.read_message(&message[..size], &mut plain).unwrap();
        let size = b.write_message(&[], &mut message).unwrap();
        a.read_message(&message[..size], &mut plain).unwrap();
        let size = a.write_message(&[], &mut message).unwrap();
        b.read_message(&message[..size], &mut plain).unwrap();
        assert_eq!(a.get_handshake_hash(), b.get_handshake_hash());
        (
            a.into_transport_mode().unwrap(),
            b.into_transport_mode().unwrap(),
        )
    }
    #[test]
    fn encrypted_transport_rejects_tampering_and_replay() {
        let (mut a, mut b) = transports();
        let mut encrypted = [0u8; 1024];
        let mut plain = [0u8; 1024];
        let size = a
            .write_message(b"sensitive file bytes", &mut encrypted)
            .unwrap();
        assert!(!encrypted[..size]
            .windows(9)
            .any(|window| window == b"sensitive"));
        let mut damaged = encrypted[..size].to_vec();
        damaged[12] ^= 0x80;
        assert!(b.read_message(&damaged, &mut plain).is_err());
        let size_plain = b.read_message(&encrypted[..size], &mut plain).unwrap();
        assert_eq!(&plain[..size_plain], b"sensitive file bytes");
        assert!(b.read_message(&encrypted[..size], &mut plain).is_err());
    }
}
