use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "lowercase")]
pub enum ScanResult {
    Clean,
    Infected { detail: String },
    Error { detail: String },
    Skipped,
}

fn defender_path() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let platform = Path::new(r"C:\ProgramData\Microsoft\Windows Defender\Platform");
    let newest = std::fs::read_dir(platform)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("MpCmdRun.exe"))
        .filter(|p| p.exists())
        .max();
    newest.or_else(|| {
        Some(PathBuf::from(
            r"C:\Program Files\Windows Defender\MpCmdRun.exe",
        ))
        .filter(|p| p.exists())
    })
}

fn find_in_path(exe: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(exe))
            .find(|p| p.is_file())
    })
}

pub fn should_scan(extensions: &[String], file: &Path) -> bool {
    if extensions.is_empty() {
        return true;
    }
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    extensions
        .iter()
        .any(|item| item.trim().trim_start_matches('.').eq_ignore_ascii_case(&ext))
}

pub async fn scan_file(kind: &str, custom_cmd: Option<&str>, file: &Path) -> ScanResult {
    if !file.is_file() {
        return ScanResult::Skipped;
    }

    let resolved_kind = match kind.trim().to_ascii_lowercase().as_str() {
        "defender" => "defender",
        "clamav" => "clamav",
        "custom" => "custom",
        _ => {
            if cfg!(windows) {
                "defender"
            } else {
                "clamav"
            }
        }
    };

    let (bin, args): (PathBuf, Vec<String>) = match resolved_kind {
        "defender" => match defender_path() {
            Some(p) => (
                p,
                vec![
                    "-Scan".into(),
                    "-ScanType".into(),
                    "3".into(),
                    "-File".into(),
                    file.to_string_lossy().into_owned(),
                    "-DisableRemediation".into(),
                ],
            ),
            None => {
                return ScanResult::Error {
                    detail: "Windows Defender MpCmdRun.exe not found".into(),
                }
            }
        },
        "clamav" => {
            let exe = if cfg!(windows) { "clamscan.exe" } else { "clamscan" };
            match find_in_path(exe) {
                Some(p) => (
                    p,
                    vec!["--no-summary".into(), file.to_string_lossy().into_owned()],
                ),
                None => {
                    return ScanResult::Error {
                        detail: "clamscan not found in PATH".into(),
                    }
                }
            }
        }
        "custom" => {
            let cmd = custom_cmd.map(str::trim).unwrap_or_default();
            if cmd.is_empty() {
                return ScanResult::Skipped;
            }
            (PathBuf::from(cmd), vec![file.to_string_lossy().into_owned()])
        }
        _ => return ScanResult::Skipped,
    };

    let mut command = Command::new(&bin);
    command.args(&args);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    match command.output().await {
        Ok(out) => {
            let code = out.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            match code {
                0 => ScanResult::Clean,
                // Defender returns 2 on threat found; ClamAV returns 1 on virus found.
                1 | 2 => ScanResult::Infected {
                    detail: if !stdout.is_empty() {
                        stdout
                    } else if !stderr.is_empty() {
                        stderr
                    } else {
                        format!("Scanner flagged file (exit code {code})")
                    },
                },
                _ => ScanResult::Error {
                    detail: if !stderr.is_empty() {
                        stderr
                    } else {
                        format!("Scanner exited with code {code}")
                    },
                },
            }
        }
        Err(e) => ScanResult::Error {
            detail: format!("Failed to launch scanner: {e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_filter_matches_case_insensitively() {
        let exts = vec!["exe".into(), ".ZIP".into()];
        assert!(should_scan(&exts, Path::new("setup.EXE")));
        assert!(should_scan(&exts, Path::new("archive.zip")));
        assert!(!should_scan(&exts, Path::new("readme.txt")));
        assert!(should_scan(&[], Path::new("anything.bin")));
    }
}
