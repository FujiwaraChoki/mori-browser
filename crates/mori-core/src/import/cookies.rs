//! Cookie values from another browser.
//!
//! Firefox stores them in the clear. Chrome-family browsers encrypt each
//! value with AES-128-CBC: the key is PBKDF2-HMAC-SHA1 of a password over
//! the salt `saltysalt`, the IV is sixteen spaces, and the ciphertext is
//! prefixed with `v10` or `v11`. On Linux `v10` uses the fixed password
//! `peanuts` and `v11` uses a random password from the keyring. On macOS
//! everything is `v10` and the password comes from the login keychain
//! (PBKDF2 runs 1003 times there, once on Linux).

use std::cell::RefCell;

use zeroize::Zeroize;

use super::BrowserKind;

/// How many times PBKDF2 hashes the cookie password.
const LINUX_ITERATIONS: u32 = 1;
const MACOS_ITERATIONS: u32 = 1003;
const SALT: &[u8] = b"saltysalt";
const IV: [u8; 16] = *b"                ";

/// Passwords that open a Chrome-family cookie database. Derived keys are
/// cached and wiped on drop.
pub(super) struct ChromiumCrypto {
    password: Option<Vec<u8>>,
    iterations: u32,
    /// macOS encrypts `v10` with the keychain password. Linux `v10` is `peanuts`.
    v10_uses_password: bool,
    keys: RefCell<Option<Keys>>,
}

struct Keys {
    v10: Option<[u8; 16]>,
    v11: Option<[u8; 16]>,
}

impl Drop for Keys {
    fn drop(&mut self) {
        if let Some(key) = &mut self.v10 {
            key.zeroize();
        }
        if let Some(key) = &mut self.v11 {
            key.zeroize();
        }
    }
}

impl Drop for ChromiumCrypto {
    fn drop(&mut self) {
        if let Some(password) = &mut self.password {
            password.zeroize();
        }
    }
}

impl ChromiumCrypto {
    /// Crypto for this machine: keychain on macOS, keyring (and `peanuts`) on Linux.
    pub(super) fn for_browser(kind: BrowserKind) -> ChromiumCrypto {
        ChromiumCrypto::new(password_for(kind), cfg!(target_os = "macos"))
    }

    pub(super) fn new(password: Option<Vec<u8>>, macos: bool) -> ChromiumCrypto {
        ChromiumCrypto {
            password,
            iterations: if macos { MACOS_ITERATIONS } else { LINUX_ITERATIONS },
            v10_uses_password: macos,
            keys: RefCell::new(None),
        }
    }

    /// The cookie value inside `encrypted_value`, when it can be opened.
    pub(super) fn decrypt(&self, blob: &[u8]) -> Option<String> {
        if blob.len() < 3 + 16 {
            return None;
        }
        let (prefix, data) = blob.split_at(3);
        if data.len() % 16 != 0 {
            return None;
        }
        let mut key = match prefix {
            b"v10" => self.key(false)?,
            b"v11" => self.key(true)?,
            _ => return None,
        };
        let plain = aes128_cbc_decrypt(&key, data);
        key.zeroize();
        plain.as_deref().and_then(cookie_text)
    }

    fn key(&self, v11: bool) -> Option<[u8; 16]> {
        if self.keys.borrow().is_none() {
            *self.keys.borrow_mut() = Some(self.derive());
        }
        self.keys.borrow().as_ref().and_then(|keys| if v11 { keys.v11 } else { keys.v10 })
    }

    fn derive(&self) -> Keys {
        let v10 = if self.v10_uses_password {
            self.password.as_deref().map(|p| derive_key(p, self.iterations))
        } else {
            Some(derive_key(b"peanuts", self.iterations))
        };
        let v11 = self.password.as_deref().map(|p| derive_key(p, self.iterations));
        Keys { v10, v11 }
    }
}

fn derive_key(password: &[u8], iterations: u32) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, SALT, iterations, &mut key);
    key
}

fn aes128_cbc_decrypt(key: &[u8; 16], data: &[u8]) -> Option<Vec<u8>> {
    use aes::cipher::{BlockDecrypt, KeyInit};
    let cipher = aes::Aes128::new_from_slice(key).ok()?;
    let mut prev = IV;
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.as_chunks::<16>().0 {
        let mut block = aes::cipher::generic_array::GenericArray::clone_from_slice(chunk);
        cipher.decrypt_block(&mut block);
        for (i, byte) in block.iter().enumerate() {
            out.push(byte ^ prev[i]);
        }
        prev.copy_from_slice(chunk);
    }
    let len = pkcs7_len(&out)?;
    out.truncate(len);
    Some(out)
}

fn pkcs7_len(data: &[u8]) -> Option<usize> {
    let n = *data.last()? as usize;
    if n == 0 || n > 16 || data.len() < n {
        return None;
    }
    data[data.len() - n..].iter().all(|b| *b as usize == n).then_some(data.len() - n)
}

/// Cookie values are text. Some Chrome builds prepend 32 bytes (a host
/// hash) that is not text; the value after it is.
fn cookie_text(plain: &[u8]) -> Option<String> {
    fn text(bytes: &[u8]) -> Option<String> {
        let s = std::str::from_utf8(bytes).ok()?;
        (!s.chars().any(|c| c.is_control())).then(|| s.to_string())
    }
    text(plain).or_else(|| plain.get(32..).and_then(text))
}

/// `SameSite=None` without `Secure` is rejected by Chromium. Lax still
/// signs the user in.
pub(super) fn same_site(value: i64, secure: bool) -> super::SameSite {
    let site = match value {
        0 => super::SameSite::None,
        1 => super::SameSite::Lax,
        2 => super::SameSite::Strict,
        _ => super::SameSite::Unspecified,
    };
    if site == super::SameSite::None && !secure { super::SameSite::Lax } else { site }
}

/// Column names of `table`, compared case-insensitively by [`has_column`].
pub(super) fn column_names(conn: &rusqlite::Connection, table: &str) -> anyhow::Result<std::collections::HashSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt.query_map([], |r| r.get::<_, String>(1))?;
    Ok(names.filter_map(Result::ok).collect())
}

pub(super) fn has_column(cols: &std::collections::HashSet<String>, name: &str) -> bool {
    cols.iter().any(|c| c.eq_ignore_ascii_case(name))
}

/// `name` when that column exists, otherwise a SQL literal (`0`, `''`, `X''`).
pub(super) fn sql_col<'a>(cols: &std::collections::HashSet<String>, name: &'a str, fallback: &'a str) -> &'a str {
    if has_column(cols, name) { name } else { fallback }
}

/// The password Chrome encrypted cookies with, if the OS will give it up.
fn password_for(kind: BrowserKind) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        macos_password(kind)
    }
    #[cfg(target_os = "linux")]
    {
        linux_password(kind)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = kind;
        None
    }
}

#[cfg(target_os = "macos")]
fn macos_password(kind: BrowserKind) -> Option<Vec<u8>> {
    let (service, account) = match kind {
        BrowserKind::Chrome => ("Chrome Safe Storage", "Chrome"),
        BrowserKind::Chromium => ("Chromium Safe Storage", "Chromium"),
        BrowserKind::Brave => ("Brave Safe Storage", "Brave"),
        BrowserKind::Vivaldi => ("Vivaldi Safe Storage", "Vivaldi"),
        BrowserKind::Edge => ("Microsoft Edge Safe Storage", "Microsoft Edge"),
        _ => return None,
    };
    // `-w` prints the password. It is not an argument, so it stays out of
    // the process list. macOS asks the user the first time.
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-s", service, "-a", account, "-w"])
        .output()
        .ok()?;
    let mut bytes = output.stdout;
    while bytes.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
        bytes.pop();
    }
    if !output.status.success() || bytes.is_empty() {
        bytes.zeroize();
        log::debug!("import: no keychain item \"{service}\" ({})", output.status);
        return None;
    }
    Some(bytes)
}

#[cfg(target_os = "linux")]
fn linux_password(kind: BrowserKind) -> Option<Vec<u8>> {
    let apps: Vec<&str> = match kind {
        BrowserKind::Chrome => vec!["chrome"],
        BrowserKind::Chromium => vec!["chromium"],
        BrowserKind::Brave => vec!["brave"],
        BrowserKind::Vivaldi => vec!["vivaldi"],
        BrowserKind::Edge => vec!["microsoft-edge", "edge"],
        _ => return None,
    };
    // Secret Service blocks on D-Bus, and on a locked keyring until the
    // user answers. That must not run on the runtime's worker.
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("mori-import-keyring".into()).spawn(move || {
        let _ = tx.send(linux_lookup(&apps));
    });
    if spawned.is_err() {
        return linux_lookup(&apps);
    }
    rx.recv().ok().flatten()
}

#[cfg(target_os = "linux")]
fn linux_lookup(apps: &[&str]) -> Option<Vec<u8>> {
    use std::collections::HashMap;

    use secret_service::{EncryptionType, blocking::SecretService};

    let service = match SecretService::connect(EncryptionType::Dh) {
        Ok(service) => service,
        Err(e) => {
            log::debug!("import: cookie keyring unavailable: {e}");
            return None;
        }
    };
    for app in apps {
        let found = match service.search_items(HashMap::from([("application", *app)])) {
            Ok(found) => found,
            Err(e) => {
                log::debug!("import: cookie keyring search for {app} failed: {e}");
                continue;
            }
        };
        let secret = if let Some(item) = found.unlocked.first() {
            item.get_secret().ok()
        } else if let Some(item) = found.locked.first() {
            match item.unlock() {
                Ok(()) => item.get_secret().ok(),
                Err(e) => {
                    log::debug!("import: cookie keyring stayed locked: {e}");
                    None
                }
            }
        } else {
            None
        };
        if let Some(bytes) = secret.filter(|b| !b.is_empty()) {
            return Some(bytes);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(prefix: &str, hex: &str) -> Vec<u8> {
        let mut out = prefix.as_bytes().to_vec();
        out.extend((0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()));
        out
    }

    #[test]
    fn linux_v10_uses_peanuts_and_v11_the_keyring() {
        // Ciphertexts produced independently (OpenSSL AES-128-CBC).
        let peanuts = ChromiumCrypto::new(None, false);
        let value = peanuts.decrypt(&blob("v10", "816fe85c8663fc1e0def28e6975b63d7"));
        assert_eq!(value.as_deref(), Some("session=abc"));
        assert_eq!(peanuts.decrypt(&blob("v11", "b9a6ff922269c8415ac14c83a061c51f")), None);

        let keyring = ChromiumCrypto::new(Some(b"from-keychain".to_vec()), false);
        assert_eq!(
            keyring.decrypt(&blob("v11", "b9a6ff922269c8415ac14c83a061c51f")).as_deref(),
            Some("logged-in")
        );
        // v10 stays on the fixed password even when a keyring password exists.
        assert_eq!(keyring.decrypt(&blob("v10", "816fe85c8663fc1e0def28e6975b63d7")).as_deref(), Some("session=abc"));
    }

    #[test]
    fn macos_v10_uses_the_keychain_password() {
        let crypto = ChromiumCrypto::new(Some(b"from-keychain".to_vec()), true);
        assert_eq!(
            crypto.decrypt(&blob("v10", "a767352da41097e4f80a3211c9b6c5e9")).as_deref(),
            Some("logged-in")
        );
        assert_eq!(crypto.decrypt(&blob("v10", "816fe85c8663fc1e0def28e6975b63d7")), None);
        assert_eq!(ChromiumCrypto::new(None, true).decrypt(&blob("v10", "a767352da41097e4f80a3211c9b6c5e9")), None);
    }

    #[test]
    fn a_host_hash_before_the_value_is_skipped() {
        let crypto = ChromiumCrypto::new(None, false);
        let mut plain = vec![0xff; 32];
        plain.extend(b"session=abc");
        let padded = pkcs7_pad(&plain);
        let encrypted = aes128_cbc_encrypt(&derive_key(b"peanuts", 1), &padded);
        let mut blob = b"v10".to_vec();
        blob.extend(encrypted);
        assert_eq!(crypto.decrypt(&blob).as_deref(), Some("session=abc"));
    }

    #[test]
    fn none_without_secure_becomes_lax() {
        assert_eq!(same_site(0, false), super::super::SameSite::Lax);
        assert_eq!(same_site(0, true), super::super::SameSite::None);
        assert_eq!(same_site(2, false), super::super::SameSite::Strict);
        assert_eq!(same_site(-1, true), super::super::SameSite::Unspecified);
    }

    fn pkcs7_pad(data: &[u8]) -> Vec<u8> {
        let n = 16 - (data.len() % 16);
        let mut out = data.to_vec();
        out.extend(std::iter::repeat_n(n as u8, n));
        out
    }

    fn aes128_cbc_encrypt(key: &[u8; 16], data: &[u8]) -> Vec<u8> {
        use aes::cipher::{BlockEncrypt, KeyInit};
        let cipher = aes::Aes128::new_from_slice(key).unwrap();
        let mut prev = IV;
        let mut out = Vec::with_capacity(data.len());
        for chunk in data.as_chunks::<16>().0 {
            let mut block = aes::cipher::generic_array::GenericArray::clone_from_slice(chunk);
            for (i, byte) in block.iter_mut().enumerate() {
                *byte ^= prev[i];
            }
            cipher.encrypt_block(&mut block);
            prev.copy_from_slice(&block);
            out.extend(block);
        }
        out
    }
}
