//! Secrets at rest (stream keys). On Windows they are encrypted with DPAPI, scoped to the
//! current user, and stored as `dpapi:<hex>`. On Linux they are sealed with a key kept in the
//! Secret Service (GNOME Keyring, KWallet) and stored as `keyring:<hex>`; without a Secret
//! Service they aren't saved and last for the session only.

/// Whether saved secrets survive a restart on this system.
pub fn persistent() -> bool {
    #[cfg(windows)]
    return true;
    #[cfg(target_os = "linux")]
    return keyring::key().is_some();
    #[allow(unreachable_code)]
    false
}

/// Where saved secrets go, for the settings hint.
pub fn storage_note() -> &'static str {
    if cfg!(windows) {
        "Stored encrypted for your Windows account"
    } else if persistent() {
        "Stored encrypted with your system keyring"
    } else {
        "Not saved: no system keyring is running, so it lasts until Clutch closes"
    }
}

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
    #[cfg(target_os = "linux")]
    if let Some(key) = keyring::key() {
        return format!("keyring:{}", hex(&keyring::seal(key, plain.as_bytes())));
    }
    String::new()
}

/// Decrypts a value produced by [`protect`]. Returns an empty string if it can't.
pub fn unprotect(stored: &str) -> String {
    #[cfg(target_os = "linux")]
    if let Some(data) = stored.strip_prefix("keyring:").and_then(unhex) {
        let Some(key) = keyring::key() else {
            return String::new();
        };
        return match keyring::open(key, &data) {
            Some(bytes) => String::from_utf8(bytes).unwrap_or_default(),
            None => {
                tracing::warn!("couldn't decrypt the saved stream key (keyring was reset?)");
                String::new()
            }
        };
    }
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

#[cfg(any(windows, target_os = "linux"))]
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

/// One random key in the Secret Service seals every secret (ChaCha20-Poly1305), so the
/// settings file holds no plain text and the keyring holds a single entry.
#[cfg(target_os = "linux")]
mod keyring {
    use keyring_core::Error;
    use keyring_core::api::CredentialStoreApi;
    use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
    use std::sync::OnceLock;

    pub type Key = [u8; 32];

    /// The key, loaded (or made) on first use. `None` without a Secret Service.
    pub fn key() -> Option<&'static Key> {
        static KEY: OnceLock<Option<Key>> = OnceLock::new();
        KEY.get_or_init(|| {
            load_or_create()
                .inspect_err(
                    |e| tracing::warn!(error = %e, "no system keyring; stream keys won't be saved"),
                )
                .ok()
        })
        .as_ref()
    }

    fn load_or_create() -> Result<Key, Error> {
        let store = zbus_secret_service_keyring_store::Store::new()?;
        let entry = store.build("Clutch", "secrets", None)?;
        match entry.get_secret() {
            Ok(bytes) => bytes
                .try_into()
                .map_err(|_| Error::Invalid("secret".into(), "not 32 bytes".into())),
            Err(Error::NoEntry) => {
                let mut key = [0u8; 32];
                getrandom::fill(&mut key).map_err(|e| {
                    Error::PlatformFailure(Box::new(std::io::Error::other(e.to_string())))
                })?;
                entry.set_secret(&key)?;
                Ok(key)
            }
            Err(e) => Err(e),
        }
    }

    fn cipher(key: &Key) -> LessSafeKey {
        // A 32-byte key is always valid for ChaCha20-Poly1305.
        #[allow(clippy::expect_used)]
        LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, key).expect("32-byte key"))
    }

    /// `nonce || ciphertext || tag`, with a random nonce.
    pub fn seal(key: &Key, plain: &[u8]) -> Vec<u8> {
        let mut nonce = [0u8; NONCE_LEN];
        if getrandom::fill(&mut nonce).is_err() {
            nonce = std::array::from_fn(|_| fastrand::u8(..));
        }
        let mut out = nonce.to_vec();
        let mut data = plain.to_vec();
        // Only fails when the input is larger than the cipher allows (~256 GB).
        let _ = cipher(key).seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::empty(),
            &mut data,
        );
        out.extend_from_slice(&data);
        out
    }

    /// Opens what [`seal`] made; `None` if it was made with another key or altered.
    pub fn open(key: &Key, sealed: &[u8]) -> Option<Vec<u8>> {
        let (nonce, data) = sealed.split_first_chunk::<NONCE_LEN>()?;
        let mut data = data.to_vec();
        let plain = cipher(key)
            .open_in_place(
                Nonce::assume_unique_for_key(*nonce),
                Aad::empty(),
                &mut data,
            )
            .ok()?;
        Some(plain.to_vec())
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

    #[cfg(target_os = "linux")]
    #[test]
    fn keyring_seal_round_trips_and_rejects_others() {
        let key = [7u8; 32];
        let sealed = keyring::seal(&key, b"live_123");
        assert!(!sealed.windows(8).any(|w| w == b"live_123"));
        assert_ne!(
            sealed,
            keyring::seal(&key, b"live_123"),
            "fresh nonce each time"
        );
        assert_eq!(keyring::open(&key, &sealed).unwrap(), b"live_123");
        assert!(keyring::open(&[8u8; 32], &sealed).is_none(), "another key");
        let mut altered = sealed.clone();
        *altered.last_mut().unwrap() ^= 1;
        assert!(keyring::open(&key, &altered).is_none(), "altered");
        assert!(keyring::open(&key, &[1, 2, 3]).is_none(), "too short");
    }
}
