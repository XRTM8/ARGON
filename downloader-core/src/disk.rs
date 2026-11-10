use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::sync::{mpsc, oneshot};

pub const WRITE_BUFFER_CAP: usize = 512 * 1024;
pub const COALESCED_FLUSH_THRESHOLD: usize = 256 * 1024;
const MAX_POOLED_BUFFERS: usize = 16;

fn write_all_at(file: &std::fs::File, mut offset: u64, mut data: &[u8]) -> io::Result<()> {
    #[cfg(windows)]
    use std::os::windows::fs::FileExt;
    #[cfg(unix)]
    use std::os::unix::fs::FileExt;

    while !data.is_empty() {
        #[cfg(windows)]
        let written = file.seek_write(data, offset)?;
        #[cfg(unix)]
        let written = file.write_at(data, offset)?;

        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "failed to write whole buffer",
            ));
        }
        data = &data[written..];
        offset += written as u64;
    }
    Ok(())
}

enum WriteCmd {
    Truncate { reply: oneshot::Sender<Result<(), String>> },
    Write { offset: u64, buf: Vec<u8> },
    Flush { reply: oneshot::Sender<Result<(), String>> },
    Sync { reply: oneshot::Sender<Result<(), String>> },
}

/// Shared single-file disk writer: one OS thread and one file handle shared across all segments of a task.
pub struct CoalescedDiskWriter {
    tx: mpsc::Sender<WriteCmd>,
    pool: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl CoalescedDiskWriter {
    pub fn open(path: &Path) -> Result<Arc<Self>, String> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|e| format!("Cannot open destination: {e}"))?;
        let (tx, mut rx) = mpsc::channel::<WriteCmd>(32);
        let pool = Arc::new(Mutex::new(Vec::<Vec<u8>>::with_capacity(MAX_POOLED_BUFFERS)));
        let pool_worker = pool.clone();

        std::thread::Builder::new()
            .name("argon-disk".into())
            .spawn(move || {
                let mut sticky_err: Option<String> = None;
                while let Some(cmd) = rx.blocking_recv() {
                    match cmd {
                        WriteCmd::Truncate { reply } => {
                            let res = match file.set_len(0) {
                                Ok(()) => Ok(()),
                                Err(e) => {
                                    let msg = format!("Cannot reset destination: {e}");
                                    sticky_err = Some(msg.clone());
                                    Err(msg)
                                }
                            };
                            let _ = reply.send(res);
                        }
                        WriteCmd::Write { offset, mut buf } => {
                            if sticky_err.is_none() && !buf.is_empty() {
                                if let Err(e) = write_all_at(&file, offset, &buf) {
                                    sticky_err = Some(format!("Disk write failed: {e}"));
                                }
                            }
                            buf.clear();
                            if buf.capacity() <= WRITE_BUFFER_CAP * 2 {
                                if let Ok(mut guard) = pool_worker.lock() {
                                    if guard.len() < MAX_POOLED_BUFFERS {
                                        guard.push(buf);
                                    }
                                }
                            }
                        }
                        WriteCmd::Flush { reply } => {
                            let res = if let Some(ref err) = sticky_err {
                                Err(err.clone())
                            } else {
                                use std::io::Write;
                                (&file)
                                    .flush()
                                    .map_err(|e| format!("Disk flush failed: {e}"))
                            };
                            let _ = reply.send(res);
                        }
                        WriteCmd::Sync { reply } => {
                            let res = if let Some(ref err) = sticky_err {
                                Err(err.clone())
                            } else {
                                use std::io::Write;
                                (&file)
                                    .flush()
                                    .map_err(|e| format!("Disk flush failed: {e}"))
                                    .and_then(|()| {
                                        file.sync_all()
                                            .map_err(|e| format!("Disk sync failed: {e}"))
                                    })
                            };
                            let _ = reply.send(res);
                        }
                    }
                }
            })
            .map_err(|e| format!("Cannot start disk writer: {e}"))?;

        Ok(Arc::new(Self { tx, pool }))
    }

    fn take_buffer(&self) -> Vec<u8> {
        if let Ok(mut guard) = self.pool.lock() {
            if let Some(mut buf) = guard.pop() {
                buf.clear();
                return buf;
            }
        }
        Vec::with_capacity(WRITE_BUFFER_CAP)
    }

    pub async fn truncate(&self) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(WriteCmd::Truncate { reply })
            .await
            .map_err(|_| "Disk writer stopped".to_string())?;
        rx.await.map_err(|_| "Disk writer stopped".to_string())?
    }

    pub async fn write_chunk(&self, offset: u64, buf: Vec<u8>) -> Result<(), String> {
        if buf.is_empty() {
            return Ok(());
        }
        self.tx
            .send(WriteCmd::Write { offset, buf })
            .await
            .map_err(|_| "Disk writer stopped".to_string())
    }

    pub async fn flush(&self) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(WriteCmd::Flush { reply })
            .await
            .map_err(|_| "Disk writer stopped".to_string())?;
        rx.await.map_err(|_| "Disk writer stopped".to_string())?
    }

    pub async fn sync(&self) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(WriteCmd::Sync { reply })
            .await
            .map_err(|_| "Disk writer stopped".to_string())?;
        rx.await.map_err(|_| "Disk writer stopped".to_string())?
    }
}

/// Per-segment coalescing buffer backed by a shared `CoalescedDiskWriter`.
pub struct SegmentWriter {
    disk: Arc<CoalescedDiskWriter>,
    active_buf: Vec<u8>,
    buf_start: u64,
}

impl SegmentWriter {
    pub fn new(disk: Arc<CoalescedDiskWriter>) -> Self {
        let active_buf = disk.take_buffer();
        Self {
            disk,
            active_buf,
            buf_start: 0,
        }
    }

    pub async fn truncate(&mut self) -> Result<(), String> {
        self.active_buf.clear();
        self.disk.truncate().await
    }

    pub async fn push(&mut self, offset: u64, slice: &[u8]) -> Result<(), String> {
        if slice.is_empty() {
            return Ok(());
        }
        if !self.active_buf.is_empty() && self.buf_start + self.active_buf.len() as u64 != offset {
            self.flush_buffer().await?;
        }
        if self.active_buf.is_empty() {
            self.buf_start = offset;
        }
        self.active_buf.extend_from_slice(slice);
        if self.active_buf.len() >= COALESCED_FLUSH_THRESHOLD {
            self.flush_buffer().await?;
        }
        Ok(())
    }

    pub async fn flush_buffer(&mut self) -> Result<(), String> {
        if self.active_buf.is_empty() {
            return Ok(());
        }
        let next = self.disk.take_buffer();
        let full = std::mem::replace(&mut self.active_buf, next);
        self.disk.write_chunk(self.buf_start, full).await
    }

    pub async fn flush(&mut self) -> Result<(), String> {
        self.flush_buffer().await?;
        self.disk.flush().await
    }

    pub async fn sync(&mut self) -> Result<(), String> {
        self.flush_buffer().await?;
        self.disk.sync().await
    }
}

pub fn save_json_atomic_blocking(path: &Path, value: &impl Serialize) -> Result<(), String> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
    }
    replace_file_atomic(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

pub async fn save_json_atomic(path: PathBuf, bytes: Vec<u8>) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
        {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .map_err(|e| e.to_string())?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
        }
        replace_file_atomic(&tmp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(windows)]
fn replace_file_atomic(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let src: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let dst: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    for attempt in 0..5 {
        let ok = unsafe {
            MoveFileExW(
                src.as_ptr(),
                dst.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if ok != 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::PermissionDenied && attempt < 4 {
            std::thread::sleep(std::time::Duration::from_millis(25 * (attempt + 1)));
            continue;
        }
        return Err(err);
    }
    Err(io::Error::last_os_error())
}

#[cfg(not(windows))]
fn replace_file_atomic(source: &Path, target: &Path) -> io::Result<()> {
    std::fs::rename(source, target)
}

/// Strips Windows `\\?\` verbatim UNC prefix and lowercases for reliable prefix checks.
pub fn normalize_for_prefix_check(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let stripped = if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        raw.into_owned()
    };
    stripped.to_ascii_lowercase().replace('/', "\\")
}

pub fn is_forbidden_system_path(canonical: &Path) -> bool {
    let norm = normalize_for_prefix_check(canonical);
    let unix_norm = canonical.to_string_lossy().to_ascii_lowercase();
    let forbidden_win = [
        r"c:\windows",
        r"c:\program files",
        r"c:\program files (x86)",
        r"c:\programdata",
    ];
    if forbidden_win.iter().any(|p| norm == *p || norm.starts_with(&format!("{p}\\"))) {
        return true;
    }
    let forbidden_unix = ["/etc", "/usr", "/bin", "/sbin", "/lib", "/system", "/library"];
    forbidden_unix
        .iter()
        .any(|p| unix_norm == *p || unix_norm.starts_with(&format!("{p}/")))
}

pub async fn delete_safe_file(path: &Path, allowed_roots: &[PathBuf]) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    if !path.is_file() {
        return Err("Security violation: destination must be a file, not a directory".into());
    }
    let canonical = tokio::fs::canonicalize(path)
        .await
        .map_err(|e| e.to_string())?;
    if is_forbidden_system_path(&canonical) {
        return Err("Security violation: cannot delete system protected files".into());
    }
    let in_allowed = allowed_roots.iter().any(|root| {
        root.canonicalize()
            .ok()
            .is_some_and(|c| canonical.starts_with(&c))
    });
    if !in_allowed {
        return Err("Security violation: target file is outside user allowed directories".into());
    }
    tokio::fs::remove_file(canonical)
        .await
        .map_err(|e| format!("Failed to delete file from disk: {e}"))
}

pub async fn prepare_destination_file(
    target_path: &Path,
    total_bytes: u64,
    fresh: bool,
    reset: bool,
) -> Result<PathBuf, String> {
    let mut target = target_path.to_path_buf();
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Cannot create download folder: {e}"))?;
    }
    let file = if fresh {
        let original = target.clone();
        let file_name = original
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let (stem, ext) = crate::resolver::split_filename(&file_name);
        let mut attempt = 0;
        loop {
            match tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .await
            {
                Ok(file) => break file,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    attempt += 1;
                    target = original.with_file_name(format!("{stem} ({attempt}){ext}"));
                }
                Err(e) => return Err(format!("Cannot create destination: {e}")),
            }
        }
    } else {
        tokio::fs::OpenOptions::new()
            .write(true)
            .open(&target)
            .await
            .map_err(|e| format!("Cannot resume destination (restart the download): {e}"))?
    };

    if fresh || reset {
        file.set_len(total_bytes)
            .await
            .map_err(|e| format!("Cannot allocate download file: {e}"))?;
    } else if file.metadata().await.map_err(|e| e.to_string())?.len() != total_bytes {
        return Err("Local file size changed; restart the download".into());
    }
    Ok(target)
}

pub async fn verify_file_sha256(path: &Path) -> Result<String, String> {
    if !path.exists() || !path.is_file() {
        return Err("Target file does not exist or is not a regular file".into());
    }
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut file =
            std::fs::File::open(&path).map_err(|e| format!("Cannot open file for hashing: {e}"))?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let n = file
                .read(&mut buffer)
                .map_err(|e| format!("Read error: {e}"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        Ok(format!("{:x}", hasher.finalize()))
    })
    .await
    .map_err(|e| e.to_string())?
}

pub fn local_hour() -> u8 {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{Foundation::SYSTEMTIME, System::SystemInformation::GetLocalTime};
        let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
        unsafe { GetLocalTime(&mut st) };
        st.wHour as u8
    }
    #[cfg(unix)]
    {
        unsafe {
            let mut t: libc::time_t = 0;
            libc::time(&mut t);
            let mut tm: libc::tm = std::mem::zeroed();
            libc::localtime_r(&t, &mut tm);
            tm.tm_hour.clamp(0, 23) as u8
        }
    }
    #[cfg(not(any(windows, unix)))]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_unc_forbidden_prefix_is_caught() {
        assert!(is_forbidden_system_path(Path::new(r"\\?\C:\Windows\System32\cmd.exe")));
        assert!(is_forbidden_system_path(Path::new(r"\\?\c:\Program Files\App\app.exe")));
        assert!(!is_forbidden_system_path(Path::new(r"\\?\C:\Users\Demo\Downloads\file.zip")));
    }
}
