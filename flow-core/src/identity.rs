use crate::model::hex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
use zeroize::{Zeroize, Zeroizing};

pub const NOISE: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
pub struct Identity {
    pub private: Zeroizing<Vec<u8>>,
    pub id: String,
}
#[derive(Serialize, Deserialize)]
struct DiskIdentity {
    private: Vec<u8>,
    public: Vec<u8>,
}

impl Identity {
    pub fn load_or_create(directory: &Path) -> Result<Self, String> {
        fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        let path = directory.join("identity-v2.bin");
        let disk = if path.exists() {
            let protected = fs::read(&path).map_err(|e| e.to_string())?;
            let bytes = Zeroizing::new(unprotect(&protected)?);
            serde_json::from_slice::<DiskIdentity>(&bytes).map_err(|_| {
                "Identity is damaged; do not silently replace a paired identity".to_owned()
            })?
        } else {
            let pair = snow::Builder::new(NOISE.parse().map_err(|e| format!("{e}"))?)
                .generate_keypair()
                .map_err(|e| e.to_string())?;
            let mut disk = DiskIdentity {
                private: pair.private,
                public: pair.public,
            };
            let bytes = Zeroizing::new(serde_json::to_vec(&disk).map_err(|e| e.to_string())?);
            let protected = protect(&bytes)?;
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            use std::io::Write;
            let mut file = options
                .open(&path)
                .map_err(|e| format!("Cannot save identity: {e}"))?;
            if let Err(e) = file.write_all(&protected).and_then(|_| file.sync_all()) {
                disk.private.zeroize();
                return Err(e.to_string());
            }
            disk
        };
        if disk.private.len() != 32 || disk.public.len() != 32 {
            return Err("Invalid device identity".into());
        }
        let id = fingerprint(&disk.public);
        Ok(Self {
            private: Zeroizing::new(disk.private),
            id,
        })
    }
}
pub fn fingerprint(public: &[u8]) -> String {
    hex(&Sha256::digest(public))
}

#[cfg(windows)]
fn protect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    dpapi(bytes, false)
}
#[cfg(windows)]
fn unprotect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    dpapi(bytes, true)
}
#[cfg(windows)]
fn dpapi(bytes: &[u8], decrypt: bool) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let success = unsafe {
        if decrypt {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if success == 0 {
        return Err(format!(
            "Windows could not protect/unprotect this device identity: {}",
            std::io::Error::last_os_error()
        ));
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        std::ptr::write_bytes(output.pbData, 0, output.cbData as usize);
        LocalFree(output.pbData as _);
    }
    Ok(result)
}
#[cfg(not(windows))]
fn protect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    Ok(bytes.to_vec())
}
#[cfg(not(windows))]
fn unprotect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    Ok(bytes.to_vec())
}

pub fn save_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    use std::io::Write;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(value).map_err(|e| e.to_string())?)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(&temp, path).map_err(|e| e.to_string())
}
