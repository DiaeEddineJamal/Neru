//! What Neru knows about models and free plans that changes faster than releases: how well model
//! families code, the daily allowances free plans document, and the keyless starter endpoint. The copy
//! in `registry/models.json` is built in; a newer one, signed with the same key as app updates, is
//! fetched from the `registry` release when the app starts and kept for later runs.

use std::{
    collections::HashMap,
    sync::{LazyLock, RwLock},
    time::Duration,
};

use serde::Deserialize;

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    /// Raised with every published change; an older copy never replaces a newer one.
    pub version: u64,
    #[serde(default)]
    pub updated: String,
    /// Model-name fragments and how well that family codes (0 to 100), first match wins.
    pub coding_families: Vec<(String, i64)>,
    /// Requests per day free plans document but do not report in headers.
    #[serde(default)]
    pub daily_limits: HashMap<String, u64>,
    /// The keyless endpoint a first message goes to before any key is added.
    #[serde(default)]
    pub starter_base_url: Option<String>,
}

const BUILT_IN: &str = include_str!("../../../registry/models.json");
const URL: &str = "https://github.com/DiaeEddineJamal/Neru/releases/download/registry/models.json";

static CURRENT: LazyLock<RwLock<Registry>> = LazyLock::new(|| RwLock::new(initial()));

fn built_in() -> Registry {
    serde_json::from_str(BUILT_IN).expect("registry/models.json is valid")
}

fn cache_paths() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    // Tests must not read or write the user's data folder.
    if cfg!(test) {
        return None;
    }
    let dir = crate::workspace::data_dir().ok()?;
    Some((dir.join("model-registry.json"), dir.join("model-registry.json.sig")))
}

/// The built-in copy, or the one fetched earlier when it is newer and its signature still checks out.
fn initial() -> Registry {
    let built_in = built_in();
    let cached = cache_paths().and_then(|(json, sig)| Some((std::fs::read(json).ok()?, std::fs::read_to_string(sig).ok()?)));
    match cached.and_then(|(json, sig)| verify(&json, &sig, &update_key()).ok()) {
        Some(cached) if cached.version > built_in.version => cached,
        _ => built_in,
    }
}

/// The public key app updates are signed with (tauri.conf.json), which signs the registry too.
fn update_key() -> String {
    static KEY: LazyLock<String> = LazyLock::new(|| {
        let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap_or_default();
        config["plugins"]["updater"]["pubkey"].as_str().unwrap_or_default().to_string()
    });
    KEY.clone()
}

/// Checks `json` against a Tauri-style signature (base64 of a minisign signature file) made with the
/// key whose base64 public-key file is `public_key`, then reads it.
pub fn verify(json: &[u8], signature: &str, public_key: &str) -> Result<Registry, String> {
    use base64::Engine;
    let decode = |text: &str| base64::engine::general_purpose::STANDARD.decode(text.trim()).map_err(|e| e.to_string()).and_then(|bytes| String::from_utf8(bytes).map_err(|e| e.to_string()));
    let key = minisign_verify::PublicKey::decode(&decode(public_key)?).map_err(|e| format!("bad public key: {e}"))?;
    let signature = minisign_verify::Signature::decode(&decode(signature)?).map_err(|e| format!("bad signature: {e}"))?;
    key.verify(json, &signature, false).map_err(|e| format!("the signature does not match: {e}"))?;
    serde_json::from_slice(json).map_err(|e| format!("the registry cannot be read: {e}"))
}

/// A copy of the registry in use.
pub fn get() -> Registry {
    with(Registry::clone)
}

/// Reads the registry in use without copying it, for lookups in hot loops (model sorting).
pub fn with<T>(read: impl FnOnce(&Registry) -> T) -> T {
    match CURRENT.read() {
        Ok(registry) => read(&registry),
        Err(_) => read(&built_in()),
    }
}

/// Fetches the published registry and uses it when it is signed and newer. Quiet on any failure: the
/// copy in use keeps working.
pub async fn refresh() {
    let client = crate::providers::http();
    let fetch = |url: String| {
        let client = client.clone();
        async move {
            let response = client.get(url).timeout(Duration::from_secs(15)).send().await.ok()?;
            if !response.status().is_success() {
                return None;
            }
            response.bytes().await.ok()
        }
    };
    let (Some(json), Some(sig)) = (fetch(URL.to_string()).await, fetch(format!("{URL}.sig")).await) else { return };
    let Ok(signature) = String::from_utf8(sig.to_vec()) else { return };
    let Ok(fetched) = verify(&json, &signature, &update_key()) else {
        log::warn!("The published model registry is not signed with the update key; keeping the current one");
        return;
    };
    if fetched.version <= get().version {
        return;
    }
    if let Some((json_path, sig_path)) = cache_paths() {
        let _ = std::fs::write(json_path, &json);
        let _ = std::fs::write(sig_path, &signature);
    }
    if let Ok(mut current) = CURRENT.write() {
        *current = fetched;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A key pair made only for these tests (`tauri signer generate`); the private half was thrown away.
    const TEST_KEY: &str = include_str!("../tests/fixtures/registry/test-key.pub");
    const SIGNED: &[u8] = include_bytes!("../tests/fixtures/registry/models.json");
    const SIGNATURE: &str = include_str!("../tests/fixtures/registry/models.json.sig");

    #[test]
    fn the_built_in_registry_reads() {
        let registry = built_in();
        assert!(registry.version >= 1);
        assert!(registry.coding_families.iter().any(|(family, _)| family == "qwen3-coder"));
        assert_eq!(registry.daily_limits.get("modelscope"), Some(&2_000));
        assert!(!update_key().is_empty(), "the update key is read from tauri.conf.json");
    }

    #[test]
    fn only_a_signed_registry_is_accepted() {
        let registry = verify(SIGNED, SIGNATURE, TEST_KEY).expect("signed with the test key");
        assert!(registry.version >= 1);
        let mut tampered = SIGNED.to_vec();
        let at = tampered.iter().position(|byte| byte.is_ascii_digit()).unwrap();
        tampered[at] = if tampered[at] == b'9' { b'8' } else { tampered[at] + 1 };
        assert!(verify(&tampered, SIGNATURE, TEST_KEY).unwrap_err().contains("does not match"));
        assert!(verify(SIGNED, SIGNATURE, &update_key()).is_err(), "the test key is not the update key");
    }
}
