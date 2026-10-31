//! Secret chat keys on this device (DESK-7).
//!
//! The web app used to keep its Olm account and pickles in IndexedDB. That is a browser store:
//! it lives inside the WebView's data directory, unencrypted, and anyone with the profile can read
//! it. The desktop shell puts the keys behind the OS keychain instead.
//!
//! The keychain itself cannot hold the values: an Olm account pickle alone is ~9 KB while Windows
//! Credential Manager stops at 2560 bytes, and the web store also keeps a non-extractable
//! `CryptoKey` that has no byte form. So the keychain holds one 32-byte master key, and every value
//! becomes an AES-256-GCM file under the app's local data directory — one file per `scope/key`,
//! written atomically, readable only by this user.

use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use aes_gcm::{
    aead::{Aead, Payload},
    Aes256Gcm, KeyInit, Nonce,
};
use serde::{Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, Runtime};

/// The keychain entry: one master key for every secret of this app.
const SERVICE: &str = "app.orbit.desktop.secret-store";
const ACCOUNT: &str = "master-key-v1";
/// The scopes `PlatformSecretStore` uses; anything else is refused rather than written to disk.
const SCOPES: [&str; 3] = ["secret", "secret-chat", "secret-messages"];
const MAGIC: &[u8; 4] = b"OSEC";
const VERSION: u8 = 1;
const NONCE_LEN: usize = 12;
/// A value is a JSON document; anything larger is a bug on the caller's side, not a chat message.
const MAX_VALUE_BYTES: usize = 32 * 1024 * 1024;
const MAX_KEY_LEN: usize = 200;
/// Attachments are stored next to their message, so a single blob may be a whole file.
const MAX_BLOB_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub enum SecretError {
    /// The keychain refused: locked, denied, or the item was deleted.
    Locked,
    /// The file is there but does not decrypt or does not parse.
    Corrupt,
    /// The caller asked for something this store does not do.
    Invalid(String),
    /// No usable directory or keychain on this machine.
    Unavailable(String),
}

impl Serialize for SecretError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // The web side maps these codes to DOMException names, so the text is not for people.
        let code = match self {
            SecretError::Locked => "locked".to_string(),
            SecretError::Corrupt => "corrupt".to_string(),
            SecretError::Invalid(reason) => format!("invalid: {reason}"),
            SecretError::Unavailable(reason) => format!("unavailable: {reason}"),
        };
        serializer.serialize_str(&code)
    }
}

/// One value as it is written to disk, so `keys()` can read the name without another file.
#[derive(Serialize, serde::Deserialize)]
struct SealedValue {
    key: String,
    value: Value,
}

pub struct SecretStore {
    root: PathBuf,
    key: [u8; 32],
    /// One file is one value; the lock keeps `set` from racing `clear` for the same scope.
    gate: Mutex<()>,
}

impl SecretStore {
    pub fn open<R: Runtime>(app: &AppHandle<R>) -> Result<Self, SecretError> {
        let root = app
            .path()
            .app_local_data_dir()
            .map_err(|error| SecretError::Unavailable(error.to_string()))?
            .join("secret")
            .join("v1");
        fs::create_dir_all(&root).map_err(|error| SecretError::Unavailable(error.to_string()))?;
        restrict(&root);

        Ok(Self {
            root,
            key: master_key()?,
            gate: Mutex::new(()),
        })
    }

    /// The file for `scope/key`: the name is a hash, so the directory listing does not reveal
    /// which chats this device has.
    fn target(&self, scope: &str, key: &str) -> (PathBuf, String) {
        let mut hasher = Sha256::new();
        hasher.update(scope.as_bytes());
        hasher.update([0u8]);
        hasher.update(key.as_bytes());
        let digest = hex(&hasher.finalize());
        (self.root.join(scope).join(format!("{digest}.bin")), digest)
    }

    fn aad(scope: &str, digest: &str) -> Vec<u8> {
        format!("orbit-secret/v1\0{scope}\0{digest}").into_bytes()
    }

    fn seal(
        &self,
        scope: &str,
        digest: &str,
        sealed: &SealedValue,
    ) -> Result<Vec<u8>, SecretError> {
        let plain =
            serde_json::to_vec(sealed).map_err(|error| SecretError::Invalid(error.to_string()))?;
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|error| SecretError::Unavailable(error.to_string()))?;
        let nonce = random_nonce()?;

        let body = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: &Self::aad(scope, digest),
                },
            )
            .map_err(|_| SecretError::Corrupt)?;

        let mut bytes = Vec::with_capacity(4 + 1 + NONCE_LEN + body.len());
        bytes.extend_from_slice(MAGIC);
        bytes.push(VERSION);
        bytes.extend_from_slice(&nonce);
        bytes.extend_from_slice(&body);
        Ok(bytes)
    }

    /// Raw bytes with an explicit AAD: attachments are sealed exactly like values.
    fn seal_bytes(&self, aad: &[u8], plain: &[u8]) -> Result<Vec<u8>, SecretError> {
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|error| SecretError::Unavailable(error.to_string()))?;
        let nonce = random_nonce()?;
        let body = cipher
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: plain, aad })
            .map_err(|_| SecretError::Corrupt)?;
        let mut bytes = Vec::with_capacity(4 + 1 + NONCE_LEN + body.len());
        bytes.extend_from_slice(MAGIC);
        bytes.push(VERSION);
        bytes.extend_from_slice(&nonce);
        bytes.extend_from_slice(&body);
        Ok(bytes)
    }

    fn open_bytes_raw(&self, aad: &[u8], bytes: &[u8]) -> Result<Vec<u8>, SecretError> {
        if bytes.len() <= 4 + 1 + NONCE_LEN || &bytes[..4] != MAGIC || bytes[4] != VERSION {
            return Err(SecretError::Corrupt);
        }
        let nonce = &bytes[5..5 + NONCE_LEN];
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|error| SecretError::Unavailable(error.to_string()))?;
        cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: &bytes[5 + NONCE_LEN..],
                    aad,
                },
            )
            .map_err(|_| SecretError::Corrupt)
    }

    fn open_bytes(
        &self,
        scope: &str,
        digest: &str,
        bytes: &[u8],
    ) -> Result<SealedValue, SecretError> {
        if bytes.len() <= 4 + 1 + NONCE_LEN || &bytes[..4] != MAGIC || bytes[4] != VERSION {
            return Err(SecretError::Corrupt);
        }
        let nonce = &bytes[5..5 + NONCE_LEN];
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|error| SecretError::Unavailable(error.to_string()))?;
        let plain = cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: &bytes[5 + NONCE_LEN..],
                    aad: &Self::aad(scope, digest),
                },
            )
            .map_err(|_| SecretError::Corrupt)?;
        serde_json::from_slice(&plain).map_err(|_| SecretError::Corrupt)
    }

    fn get(&self, scope: &str, key: &str) -> Result<Option<Value>, SecretError> {
        validate(scope, key)?;
        let (path, digest) = self.target(scope, key);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(SecretError::Unavailable(error.to_string())),
        };
        let sealed = self.open_bytes(scope, &digest, &bytes)?;
        if sealed.key != key {
            return Err(SecretError::Corrupt);
        }
        Ok(Some(sealed.value))
    }

    fn set(&self, scope: &str, key: &str, value: Value) -> Result<(), SecretError> {
        validate(scope, key)?;
        if serde_json::to_vec(&value)
            .map_err(|error| SecretError::Invalid(error.to_string()))?
            .len()
            > MAX_VALUE_BYTES
        {
            return Err(SecretError::Invalid("value is too large".to_string()));
        }
        let (path, digest) = self.target(scope, key);
        let bytes = self.seal(
            scope,
            &digest,
            &SealedValue {
                key: key.to_string(),
                value,
            },
        )?;
        let _guard = self
            .gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        write_atomically(&path, &bytes)
    }

    /// Where the attachments of one value live: one file per blob, encrypted like the value.
    fn blob_dir(&self, scope: &str, key: &str) -> (PathBuf, String) {
        let (path, digest) = self.target(scope, key);
        let dir = path.with_extension("blobs");
        (dir, digest)
    }

    fn blob_put(&self, scope: &str, key: &str, name: &str, data: &[u8]) -> Result<(), SecretError> {
        validate(scope, key)?;
        validate_name(name)?;
        if data.len() > MAX_BLOB_BYTES {
            return Err(SecretError::Invalid("blob is too large".to_string()));
        }
        let (dir, digest) = self.blob_dir(scope, key);
        let bytes = self.seal_bytes(&blob_aad(scope, &digest, name), data)?;
        let _guard = self
            .gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        write_atomically(&dir.join(format!("{name}.bin")), &bytes)
    }

    fn blob_get(&self, scope: &str, key: &str, name: &str) -> Result<Option<Vec<u8>>, SecretError> {
        validate(scope, key)?;
        validate_name(name)?;
        let (dir, digest) = self.blob_dir(scope, key);
        let bytes = match fs::read(dir.join(format!("{name}.bin"))) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(SecretError::Unavailable(error.to_string())),
        };
        self.open_bytes_raw(&blob_aad(scope, &digest, name), &bytes)
            .map(Some)
    }

    fn blob_remove(&self, scope: &str, key: &str, name: &str) -> Result<(), SecretError> {
        validate(scope, key)?;
        validate_name(name)?;
        let (dir, _) = self.blob_dir(scope, key);
        let _guard = self
            .gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match fs::remove_file(dir.join(format!("{name}.bin"))) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(SecretError::Unavailable(error.to_string())),
        }
    }

    fn remove(&self, scope: &str, key: &str) -> Result<(), SecretError> {
        validate(scope, key)?;
        let (path, _) = self.target(scope, key);
        let (blobs, _) = self.blob_dir(scope, key);
        let _guard = self
            .gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // The attachments belong to the value: one removed message must not leave files behind.
        let _ = fs::remove_dir_all(blobs);
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(SecretError::Unavailable(error.to_string())),
        }
    }

    fn keys(&self, scope: &str) -> Result<Vec<String>, SecretError> {
        validate(scope, "")?;
        let dir = self.root.join(scope);
        let mut keys = Vec::new();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(keys),
            Err(error) => return Err(SecretError::Unavailable(error.to_string())),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let digest = match path.file_stem().and_then(|stem| stem.to_str()) {
                Some(digest) => digest.to_string(),
                None => continue,
            };
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            // A damaged file is reported by `get`; listing skips it instead of hiding the rest.
            if let Ok(sealed) = self.open_bytes(scope, &digest, &bytes) {
                keys.push(sealed.key);
            }
        }
        keys.sort();
        Ok(keys)
    }

    fn clear(&self, scope: &str) -> Result<(), SecretError> {
        validate(scope, "")?;
        let _guard = self
            .gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match fs::remove_dir_all(self.root.join(scope)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(SecretError::Unavailable(error.to_string())),
        }
    }
}

/// Refuses anything that could turn a key into a path or a scope into another directory.
fn validate(scope: &str, key: &str) -> Result<(), SecretError> {
    if !SCOPES.contains(&scope) {
        return Err(SecretError::Invalid(format!("unknown scope {scope}")));
    }
    if key.len() > MAX_KEY_LEN || key.contains('/') || key.contains('\\') || key.contains('\0') {
        return Err(SecretError::Invalid("bad key".to_string()));
    }
    Ok(())
}

fn blob_aad(scope: &str, digest: &str, name: &str) -> Vec<u8> {
    format!("orbit-secret/v1\0blob\0{scope}\0{digest}\0{name}").into_bytes()
}

/// A blob name comes from the page, so it may not walk out of its directory either.
fn validate_name(name: &str) -> Result<(), SecretError> {
    if name.is_empty()
        || name.len() > MAX_KEY_LEN
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(SecretError::Invalid("bad blob name".to_string()));
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

fn random_nonce() -> Result<[u8; NONCE_LEN], SecretError> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|error| SecretError::Unavailable(error.to_string()))?;
    Ok(nonce)
}

/// Writes next to the target and renames, so a crash leaves the old value, never half a file.
fn write_atomically(path: &PathBuf, bytes: &[u8]) -> Result<(), SecretError> {
    let dir = path
        .parent()
        .ok_or_else(|| SecretError::Unavailable("no directory".to_string()))?;
    fs::create_dir_all(dir).map_err(|error| SecretError::Unavailable(error.to_string()))?;
    let temporary = dir.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("value")
    ));

    let mut file = fs::File::create(&temporary)
        .map_err(|error| SecretError::Unavailable(error.to_string()))?;
    restrict(&temporary);
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| SecretError::Unavailable(error.to_string()))?;
    drop(file);

    fs::rename(&temporary, path).map_err(|error| SecretError::Unavailable(error.to_string()))
}

/// Only this user may read the keys (Unix); Windows inherits the ACL of LocalAppData.
fn restrict(path: &PathBuf) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if path.is_dir() { 0o700 } else { 0o600 };
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn master_key() -> Result<[u8; 32], SecretError> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    INIT.get_or_init(init_store)
        .as_ref()
        .map_err(|error| SecretError::Unavailable(error.clone()))?;

    let entry = keyring_core::Entry::new(SERVICE, ACCOUNT)
        .map_err(|error| SecretError::Unavailable(error.to_string()))?;

    match entry.get_secret() {
        Ok(secret) if secret.len() == 32 => {
            let mut key = [0u8; 32];
            key.copy_from_slice(&secret);
            Ok(key)
        }
        Ok(_) => Err(SecretError::Corrupt),
        Err(keyring_core::Error::NoEntry) => {
            let key = random_nonce_secret()?;
            entry.set_secret(&key).map_err(|error| locked(&error))?;
            Ok(key)
        }
        Err(error) => Err(locked(&error)),
    }
}

/// The keychain's own errors are opaque; they all mean "the key is not available right now".
fn locked(error: &impl std::fmt::Display) -> SecretError {
    eprintln!("[orbit-desktop] secret store: {error}");
    SecretError::Locked
}

fn random_nonce_secret() -> Result<[u8; 32], SecretError> {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|error| SecretError::Unavailable(error.to_string()))?;
    Ok(key)
}

#[cfg(target_os = "macos")]
fn init_store() -> Result<(), String> {
    let store = apple_native_keyring_store::keychain::Store::new().map_err(|e| e.to_string())?;
    keyring_core::set_default_store(store);
    Ok(())
}

#[cfg(target_os = "windows")]
fn init_store() -> Result<(), String> {
    let store = windows_native_keyring_store::Store::new().map_err(|e| e.to_string())?;
    keyring_core::set_default_store(store);
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn init_store() -> Result<(), String> {
    Err("the secret store needs macOS or Windows".to_string())
}

/// One command per `PlatformSecretStore` method; the web adapter calls them by name.
mod commands {
    use super::{SecretError, SecretStore};
    use serde_json::Value;
    use std::sync::Arc;
    use tauri::State;

    #[tauri::command]
    pub async fn secret_get(
        store: State<'_, Arc<SecretStore>>,
        scope: String,
        key: String,
    ) -> Result<Option<Value>, SecretError> {
        let store = Arc::clone(&store);
        tauri::async_runtime::spawn_blocking(move || store.get(&scope, &key))
            .await
            .map_err(|error| SecretError::Unavailable(error.to_string()))?
    }

    #[tauri::command]
    pub async fn secret_set(
        store: State<'_, Arc<SecretStore>>,
        scope: String,
        key: String,
        value: Value,
    ) -> Result<(), SecretError> {
        let store = Arc::clone(&store);
        tauri::async_runtime::spawn_blocking(move || store.set(&scope, &key, value))
            .await
            .map_err(|error| SecretError::Unavailable(error.to_string()))?
    }

    #[tauri::command]
    pub async fn secret_remove(
        store: State<'_, Arc<SecretStore>>,
        scope: String,
        key: String,
    ) -> Result<(), SecretError> {
        let store = Arc::clone(&store);
        tauri::async_runtime::spawn_blocking(move || store.remove(&scope, &key))
            .await
            .map_err(|error| SecretError::Unavailable(error.to_string()))?
    }

    #[tauri::command]
    pub async fn secret_keys(
        store: State<'_, Arc<SecretStore>>,
        scope: String,
    ) -> Result<Vec<String>, SecretError> {
        let store = Arc::clone(&store);
        tauri::async_runtime::spawn_blocking(move || store.keys(&scope))
            .await
            .map_err(|error| SecretError::Unavailable(error.to_string()))?
    }

    #[tauri::command]
    pub async fn secret_blob_put(
        store: State<'_, Arc<SecretStore>>,
        scope: String,
        key: String,
        name: String,
        data: String,
    ) -> Result<(), SecretError> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| SecretError::Invalid("blob is not base64".to_string()))?;
        let store = Arc::clone(&store);
        tauri::async_runtime::spawn_blocking(move || store.blob_put(&scope, &key, &name, &bytes))
            .await
            .map_err(|error| SecretError::Unavailable(error.to_string()))?
    }

    #[tauri::command]
    pub async fn secret_blob_get(
        store: State<'_, Arc<SecretStore>>,
        scope: String,
        key: String,
        name: String,
    ) -> Result<Option<String>, SecretError> {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let store = Arc::clone(&store);
        tauri::async_runtime::spawn_blocking(move || store.blob_get(&scope, &key, &name))
            .await
            .map_err(|error| SecretError::Unavailable(error.to_string()))?
            .map(|found| found.map(|bytes| STANDARD.encode(bytes)))
    }

    #[tauri::command]
    pub async fn secret_blob_remove(
        store: State<'_, Arc<SecretStore>>,
        scope: String,
        key: String,
        name: String,
    ) -> Result<(), SecretError> {
        let store = Arc::clone(&store);
        tauri::async_runtime::spawn_blocking(move || store.blob_remove(&scope, &key, &name))
            .await
            .map_err(|error| SecretError::Unavailable(error.to_string()))?
    }

    #[tauri::command]
    pub async fn secret_clear(
        store: State<'_, Arc<SecretStore>>,
        scope: String,
    ) -> Result<(), SecretError> {
        let store = Arc::clone(&store);
        tauri::async_runtime::spawn_blocking(move || store.clear(&scope))
            .await
            .map_err(|error| SecretError::Unavailable(error.to_string()))?
    }
}

pub use commands::*;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;

    fn store() -> (SecretStore, tempdir::TempDir) {
        let dir = tempdir::TempDir::new("orbit-secret").expect("temp dir");
        let store = SecretStore {
            root: dir.path().to_path_buf(),
            key: [7u8; 32],
            gate: Mutex::new(()),
        };
        (store, dir)
    }

    #[test]
    fn keeps_a_value_and_returns_it() {
        let (store, _dir) = store();
        store
            .set("secret", "account", json!({ "data": "cipher" }))
            .expect("set");

        assert_eq!(
            store.get("secret", "account").expect("get"),
            Some(json!({ "data": "cipher" }))
        );
    }

    #[test]
    fn a_missing_value_is_none_not_an_error() {
        let (store, _dir) = store();
        assert_eq!(store.get("secret", "nothing").expect("get"), None);
    }

    #[test]
    fn lists_only_the_scope_and_only_readable_files() {
        let (store, _dir) = store();
        store.set("secret-chat", "chat-a", json!(1)).expect("a");
        store.set("secret-chat", "chat-b", json!(2)).expect("b");
        store.set("secret", "account", json!(3)).expect("c");

        assert_eq!(
            store.keys("secret-chat").expect("keys"),
            vec!["chat-a".to_string(), "chat-b".to_string()]
        );

        let listed = store.keys("secret-chat").expect("keys");
        let (path, _) = store.target("secret-chat", "chat-a");
        fs::write(&path, b"broken").expect("tamper");
        assert_eq!(
            store.keys("secret-chat").expect("keys"),
            listed[1..].to_vec()
        );
        assert!(store.get("secret-chat", "chat-a").is_err());
    }

    #[test]
    fn removing_and_clearing_are_idempotent() {
        let (store, _dir) = store();
        store.set("secret-messages", "m", json!(1)).expect("set");
        store.remove("secret-messages", "m").expect("remove");
        store.remove("secret-messages", "m").expect("remove again");
        store.clear("secret-messages").expect("clear");
        store.clear("secret-messages").expect("clear again");
    }

    #[test]
    fn a_tampered_value_does_not_decrypt() {
        let (store, _dir) = store();
        store
            .set("secret", "account", json!({ "data": "x" }))
            .expect("set");
        let (path, _) = store.target("secret", "account");
        let mut bytes = fs::read(&path).expect("read");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        fs::write(&path, bytes).expect("write");

        assert!(matches!(
            store.get("secret", "account"),
            Err(SecretError::Corrupt)
        ));
    }

    #[test]
    fn refuses_unknown_scopes_and_path_like_keys() {
        let (store, _dir) = store();
        assert!(matches!(
            store.set("../escape", "k", json!(1)),
            Err(SecretError::Invalid(_))
        ));
        assert!(matches!(
            store.set("secret", "../escape", json!(1)),
            Err(SecretError::Invalid(_))
        ));
    }

    #[test]
    fn writes_only_inside_its_own_directory() {
        let (store, dir) = store();
        store.set("secret", "account", json!(1)).expect("set");
        let written = walk(dir.path());
        assert_eq!(written.len(), 1);
        assert!(written[0].starts_with(dir.path()));
        // The file name is a hash: the chat ids are not readable from the directory listing.
        let name = written[0]
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        assert!(!name.contains("account"));
    }

    fn walk(path: &std::path::Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let Ok(entries) = fs::read_dir(path) else {
            return found;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.extend(walk(&path));
            } else {
                found.push(path);
            }
        }
        found
    }

    #[test]
    fn a_value_too_large_for_a_keychain_is_still_a_file() {
        let (store, _dir) = store();
        let big = "x".repeat(20_000);
        store
            .set("secret", "account", json!({ "data": big }))
            .expect("set");
        let value = store.get("secret", "account").expect("get").expect("value");
        assert_eq!(value["data"].as_str().map(str::len), Some(20_000));
    }

    #[test]
    fn keeps_an_attachment_next_to_its_value() {
        let (store, _dir) = store();
        store
            .set("secret-messages", "chat-1", json!({ "text": "hi" }))
            .expect("set");
        store
            .blob_put("secret-messages", "chat-1", "file-1", b"attachment bytes")
            .expect("put");

        assert_eq!(
            store
                .blob_get("secret-messages", "chat-1", "file-1")
                .expect("get"),
            Some(b"attachment bytes".to_vec())
        );
        assert_eq!(
            store
                .blob_get("secret-messages", "chat-1", "nothing")
                .expect("get"),
            None
        );
    }

    #[test]
    fn removing_a_value_removes_its_attachments() {
        let (store, _dir) = store();
        store
            .set("secret-messages", "chat-1", json!(1))
            .expect("set");
        store
            .blob_put("secret-messages", "chat-1", "f", b"x")
            .expect("put");

        store.remove("secret-messages", "chat-1").expect("remove");

        assert_eq!(
            store
                .blob_get("secret-messages", "chat-1", "f")
                .expect("get"),
            None
        );
    }

    #[test]
    fn refuses_path_like_blob_names() {
        let (store, _dir) = store();
        assert!(matches!(
            store.blob_put("secret", "account", "../escape", b"x"),
            Err(SecretError::Invalid(_))
        ));
    }

    #[test]
    fn names_in_one_scope_never_collide() {
        let (store, _dir) = store();
        store
            .set("secret", "a/b", json!(1))
            .expect_err("slash is refused");
        store.set("secret", "a b", json!(2)).expect("space is fine");
        store
            .set("secret-chat", "a b", json!(3))
            .expect("other scope");
        assert_eq!(store.keys("secret").expect("keys"), vec!["a b".to_string()]);
        let seen: HashSet<_> = store
            .keys("secret-chat")
            .expect("keys")
            .into_iter()
            .collect();
        assert_eq!(seen.len(), 1);
    }
}
