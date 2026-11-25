#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Command, Stdio};
use std::time::Duration;

fn is_native_messaging_invocation() -> bool {
    std::env::args().skip(1).any(|arg| {
        arg == "--native-messaging"
            || arg.starts_with("chrome-extension://")
            || arg.starts_with("moz-extension://")
    })
}

fn handle_native_messaging() {
    let mut len_buf = [0u8; 4];
    if std::io::stdin().read_exact(&mut len_buf).is_ok() {
        let len = u32::from_le_bytes(len_buf) as usize;
        if len > 0 && len <= 65_536 {
            let mut body = vec![0u8; len];
            let _ = std::io::stdin().read_exact(&mut body);
        }
    }

    let addr: SocketAddr = ([127, 0, 0, 1], 41523).into();
    let already_running = TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok();
    if !already_running {
        if let Ok(exe) = std::env::current_exe() {
            let _ = Command::new(exe)
                .arg("--minimized")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
    }

    let reply = if already_running {
        br#"{"status":"ok","action":"wake","running":true}"#.as_slice()
    } else {
        br#"{"status":"ok","action":"wake","running":false}"#.as_slice()
    };
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(&(reply.len() as u32).to_le_bytes());
    let _ = out.write_all(reply);
    let _ = out.flush();
}

fn main() {
    if is_native_messaging_invocation() {
        handle_native_messaging();
        return;
    }
    otium_downloader_lib::run();
}
