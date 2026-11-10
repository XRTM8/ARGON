use crate::model::{hex, uid, valid_hash};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::io::AsyncWriteExt;

pub fn sanitize_filename(raw: &str) -> String {
    let leaf = raw.rsplit(['/', '\\']).next().unwrap_or("file");
    let clean: String = leaf
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(180)
        .collect();
    let clean = clean.trim().trim_matches(['.', ' ']).to_owned();
    if clean.is_empty() {
        return "received-file".into();
    }
    let stem = clean.split('.').next().unwrap_or("").to_ascii_uppercase();
    if [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ]
    .contains(&stem.as_str())
    {
        format!("_{clean}")
    } else {
        clean
    }
}

pub async fn hash_file(
    path: &Path,
    until: Option<u64>,
    cancel: &Arc<AtomicBool>,
) -> Result<(u64, Sha256), String> {
    let path = path.to_path_buf();
    let cancel = cancel.clone();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        let mut hasher = Sha256::new();
        let mut read = 0u64;
        // Single 1 MiB reusable buffer eliminates per-chunk spawn_blocking overhead
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            check_cancel(&cancel)?;
            let count = until.map_or(buf.len(), |size| {
                size.saturating_sub(read).min(buf.len() as u64) as usize
            });
            if count == 0 {
                break;
            }
            let n = file.read(&mut buf[..count]).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            read += n as u64;
        }
        if until.is_some_and(|size| size != read) {
            return Err("File changed during resume verification".into());
        }
        Ok((read, hasher))
    })
    .await
    .map_err(|e| e.to_string())?
}
pub fn digest(hasher: Sha256) -> String {
    hex(&hasher.finalize())
}
pub fn partial_id(hash: &str, name: &str) -> String {
    hex(&Sha256::digest(format!("{hash}\0{name}").as_bytes()))
}
pub fn check_cancel(cancel: &Arc<AtomicBool>) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Transfer cancelled".into())
    } else {
        Ok(())
    }
}

pub async fn validate_directory(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Choose an absolute receive directory".into());
    }
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|e| e.to_string())?;
    let canonical = tokio::fs::canonicalize(path)
        .await
        .map_err(|e| e.to_string())?;
    let probe = canonical.join(format!(".argon-check-{}", uid()));
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .await
        .map_err(|e| format!("Directory is not writable: {e}"))?;
    file.write_all(b"ARGON").await.map_err(|e| e.to_string())?;
    drop(file);
    tokio::fs::remove_file(&probe)
        .await
        .map_err(|e| e.to_string())?;
    Ok(canonical)
}

pub async fn partial_path(directory: &Path, peer: &str, hash: &str) -> Result<PathBuf, String> {
    if !valid_hash(peer) || !valid_hash(hash) {
        return Err("Invalid transfer identity".into());
    }
    let root = tokio::fs::canonicalize(directory)
        .await
        .map_err(|e| e.to_string())?;
    let partials = root.join(".argon-partials");
    tokio::fs::create_dir_all(&partials)
        .await
        .map_err(|e| e.to_string())?;
    let resolved = tokio::fs::canonicalize(&partials)
        .await
        .map_err(|e| e.to_string())?;
    if !resolved.starts_with(&root) {
        return Err("Partial directory escapes the receive folder".into());
    }
    let path = resolved.join(format!("{peer}-{hash}.part"));
    if let Ok(meta) = tokio::fs::symlink_metadata(&path).await {
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err("Unsafe partial file".into());
        }
    }
    Ok(path)
}

pub async fn commit_file(partial: &Path, directory: &Path, name: &str) -> Result<PathBuf, String> {
    let safe = sanitize_filename(name);
    let name_path = Path::new(&safe);
    let stem = name_path.file_stem().unwrap_or_default().to_string_lossy();
    let extension = name_path
        .extension()
        .map(|v| format!(".{}", v.to_string_lossy()))
        .unwrap_or_default();
    let root = tokio::fs::canonicalize(directory)
        .await
        .map_err(|e| e.to_string())?;
    for index in 0..10000 {
        let name = if index == 0 {
            safe.clone()
        } else {
            format!("{stem} ({index}){extension}")
        };
        let target = root.join(name);
        let source = partial.to_path_buf();
        let destination = target.clone();
        let result =
            tokio::task::spawn_blocking(move || move_without_overwrite(&source, &destination))
                .await
                .map_err(|e| e.to_string())?;
        match result {
            Ok(()) => return Ok(target),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Cannot commit received file: {e}")),
        }
    }
    Err("Too many files with the same name".into())
}
#[cfg(windows)]
fn move_without_overwrite(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let success = unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if success == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn move_without_overwrite(source: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::hard_link(source, target)?;
    std::fs::remove_file(source)
}

pub fn local_endpoint(address: &str) -> Result<std::net::SocketAddr, String> {
    let endpoint: std::net::SocketAddr = address.trim().parse().map_err(|_| {
        "Enter a local IPv4 address and port, for example 192.168.1.5:54127".to_owned()
    })?;
    match endpoint.ip() {
        std::net::IpAddr::V4(ip)
            if (ip.is_private() || ip.is_loopback() || ip.is_link_local())
                && !ip.is_unspecified()
                && endpoint.port() > 0 =>
        {
            Ok(endpoint)
        }
        _ => Err("Sharing is limited to private/local IPv4 networks".into()),
    }
}

pub fn ensure_space(directory: &Path, required: u64) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let path: Vec<u16> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free = 0;
        if unsafe {
            GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut free,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(format!(
                "Cannot check receive disk space: {}",
                std::io::Error::last_os_error()
            ));
        }
        if free < required.saturating_add(8 * 1024 * 1024) {
            return Err("Not enough free disk space for this file".into());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path =
            std::ffi::CString::new(directory.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
            return Err(format!(
                "Cannot check receive disk space: {}",
                std::io::Error::last_os_error()
            ));
        }
        let stats = unsafe { stats.assume_init() };
        let free = (stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64);
        if free < required.saturating_add(8 * 1024 * 1024) {
            return Err("Not enough free disk space for this file".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn untrusted_names_never_become_paths_or_windows_devices() {
        assert_eq!(sanitize_filename("../../secret.txt"), "secret.txt");
        assert_eq!(sanitize_filename("..\\Windows\\CON.txt"), "_CON.txt");
        assert_eq!(sanitize_filename("report:*?.txt"), "report___.txt");
        assert_eq!(sanitize_filename("..."), "received-file");
        assert_eq!(sanitize_filename("ملف تجريبي.pdf"), "ملف تجريبي.pdf");
        assert!(local_endpoint("8.8.8.8:54127").is_err());
        assert!(local_endpoint("127.0.0.1:54127").is_ok());
    }
}
