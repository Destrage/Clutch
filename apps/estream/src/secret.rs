//! Secrets at rest (stream keys). On Windows they are encrypted with DPAPI, scoped to the
//! current user, and stored as `dpapi:<hex>`. Other platforms don't persist them yet.

/// Encrypts `plain` for storage. Empty input (or failure) yields an empty string.
pub fn protect(plain: &str) -> String {
    if plain.is_empty() {
        return String::new();
    }
    #[cfg(windows)]
    {
        match dpapi::protect(plain.as_bytes()) {
            Ok(bytes) => return format!("dpapi:{}", hex(&bytes)),
            Err(e) => tracing::error!(error = %e, "couldn't encrypt the stream key"),
        }
    }
    String::new()
}

/// Decrypts a value produced by [`protect`]. Returns an empty string if it can't.
pub fn unprotect(stored: &str) -> String {
    let Some(data) = stored.strip_prefix("dpapi:").and_then(unhex) else {
        return String::new();
    };
    #[cfg(windows)]
    {
        match dpapi::unprotect(&data) {
            Ok(bytes) => return String::from_utf8(bytes).unwrap_or_default(),
            Err(e) => {
                tracing::warn!(error = %e, "couldn't decrypt the saved stream key (different user?)")
            }
        }
    }
    let _ = data;
    String::new()
}

#[cfg(windows)]
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(windows)]
mod dpapi {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr().cast_mut(),
        }
    }

    /// Copies a DPAPI output blob and frees it.
    fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // SAFETY: DPAPI returned `cbData` bytes at `pbData`, allocated with LocalAlloc.
        unsafe {
            let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            let _ = LocalFree(Some(HLOCAL(out.pbData.cast())));
            bytes
        }
    }

    pub fn protect(data: &[u8]) -> windows::core::Result<Vec<u8>> {
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: input blob borrows `data` for the duration of the call.
        unsafe {
            CryptProtectData(
                &blob(data),
                windows::core::w!("Clutch stream key"),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )?;
        }
        Ok(take(out))
    }

    pub fn unprotect(data: &[u8]) -> windows::core::Result<Vec<u8>> {
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: as above.
        unsafe {
            CryptUnprotectData(
                &blob(data),
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )?;
        }
        Ok(take(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_without_plain_text() {
        let key = "live_123456_SuperSecretKey";
        let stored = protect(key);
        assert!(!stored.contains(key));
        if cfg!(windows) {
            assert!(stored.starts_with("dpapi:"));
            assert_eq!(unprotect(&stored), key);
        }
        assert_eq!(protect(""), "");
        assert_eq!(unprotect("garbage"), "");
        assert_eq!(unprotect("dpapi:zz"), "");
    }
}
