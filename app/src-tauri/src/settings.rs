//! Settings that survive a restart: the chosen provider, model and voice endpoint, plus API keys.
//! Keys are encrypted with Windows DPAPI (bound to the signed-in Windows user) before they touch disk.

use std::{collections::HashMap, fs};

use serde::{Deserialize, Serialize};

use crate::{AppState, ProviderConfig, voice::VoiceConfig, workspace::data_dir};

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    provider_id: String,
    api_format: String,
    base_url: String,
    model: String,
    /// "provider\nbase_url" -> protected key (hex).
    keys: HashMap<String, String>,
    voice_base_url: String,
    voice_model: String,
    voice_key: String,
}

const FILE: &str = "settings.json";

/// Restores the saved provider, keys and voice settings into a fresh state.
pub fn load(provider: &mut ProviderConfig, keys: &mut HashMap<String, String>, voice: &mut VoiceConfig) {
    let Some(stored) = data_dir()
        .ok()
        .and_then(|dir| fs::read_to_string(dir.join(FILE)).ok())
        .and_then(|text| serde_json::from_str::<Stored>(&text).ok())
    else {
        return;
    };
    for (id, sealed) in stored.keys {
        if let Some(key) = secret::open(&sealed) {
            keys.insert(id, key);
        }
    }
    if !stored.provider_id.is_empty() && !stored.base_url.is_empty() {
        provider.api_key = keys
            .get(&format!("{}\n{}", stored.provider_id, stored.base_url))
            .cloned()
            .unwrap_or_default();
        provider.provider_id = stored.provider_id;
        provider.api_format = stored.api_format;
        provider.base_url = stored.base_url;
        provider.model = stored.model;
    }
    if !stored.voice_model.is_empty() {
        voice.base_url = stored.voice_base_url;
        voice.model = stored.voice_model;
        voice.api_key = secret::open(&stored.voice_key).unwrap_or_default();
    }
}

/// Writes the current provider, keys and voice settings. Keys that cannot be protected are not saved.
pub fn save(state: &AppState) -> Result<(), String> {
    let provider = state.provider.lock().map_err(|e| e.to_string())?.clone();
    let keys = state.provider_keys.lock().map_err(|e| e.to_string())?.clone();
    let voice = state.voice.lock().map_err(|e| e.to_string())?.clone();
    let stored = Stored {
        provider_id: provider.provider_id,
        api_format: provider.api_format,
        base_url: provider.base_url,
        model: provider.model,
        keys: keys
            .into_iter()
            .filter_map(|(id, key)| secret::seal(&key).map(|sealed| (id, sealed)))
            .collect(),
        voice_base_url: voice.base_url,
        voice_model: voice.model,
        voice_key: if voice.api_key.is_empty() {
            String::new()
        } else {
            secret::seal(&voice.api_key).unwrap_or_default()
        },
    };
    let path = data_dir()?.join(FILE);
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, serde_json::to_vec_pretty(&stored).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    fs::rename(&temp, &path).map_err(|e| e.to_string())
}

/// Forgets every saved key (Settings → Model provider → Forget saved keys).
#[tauri::command]
pub fn forget_keys(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.provider_keys.lock().map_err(|e| e.to_string())?.clear();
    state.provider.lock().map_err(|e| e.to_string())?.api_key.clear();
    state.voice.lock().map_err(|e| e.to_string())?.api_key.clear();
    save(&state)
}

/// Encrypts a secret for storage (Windows DPAPI); None when this platform cannot protect it.
pub fn seal_secret(value: &str) -> Option<String> {
    secret::seal(value)
}

/// Decrypts a value produced by [`seal_secret`].
pub fn open_secret(sealed: &str) -> Option<String> {
    secret::open(sealed)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(text.get(index..index + 2)?, 16).ok())
        .collect()
}

#[cfg(windows)]
mod secret {
    use std::{ffi::c_void, ptr};

    #[repr(C)]
    struct Blob {
        len: u32,
        data: *mut u8,
    }

    #[link(name = "crypt32")]
    unsafe extern "system" {
        fn CryptProtectData(
            input: *const Blob,
            description: *const u16,
            entropy: *const Blob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
        fn CryptUnprotectData(
            input: *const Blob,
            description: *mut *mut u16,
            entropy: *const Blob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    const UI_FORBIDDEN: u32 = 0x1;
    const ENTROPY: &[u8] = b"neru.provider-keys.v1";

    fn run(input: &[u8], protect: bool) -> Option<Vec<u8>> {
        let input = Blob { len: input.len().try_into().ok()?, data: input.as_ptr() as *mut u8 };
        let entropy = Blob { len: ENTROPY.len() as u32, data: ENTROPY.as_ptr() as *mut u8 };
        let mut output = Blob { len: 0, data: ptr::null_mut() };
        // SAFETY: every blob points at live memory for the duration of the call; the output buffer is
        // allocated by the OS, copied out, then released with LocalFree exactly once.
        unsafe {
            let ok = if protect {
                CryptProtectData(&input, ptr::null(), &entropy, ptr::null_mut(), ptr::null_mut(), UI_FORBIDDEN, &mut output)
            } else {
                CryptUnprotectData(&input, ptr::null_mut(), &entropy, ptr::null_mut(), ptr::null_mut(), UI_FORBIDDEN, &mut output)
            };
            if ok == 0 || output.data.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(output.data, output.len as usize).to_vec();
            LocalFree(output.data.cast());
            Some(bytes)
        }
    }

    pub fn seal(secret: &str) -> Option<String> {
        run(secret.as_bytes(), true).map(|bytes| super::to_hex(&bytes))
    }

    pub fn open(sealed: &str) -> Option<String> {
        String::from_utf8(run(&super::from_hex(sealed)?, false)?).ok()
    }
}

/// macOS Keychain and the Linux Secret Service (GNOME Keyring, KWallet) through their standard
/// command-line tools. The settings file only stores a random item id; the secret stays in the
/// system keychain. Without a keychain, keys are kept for the session only.
#[cfg(not(windows))]
mod secret {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };

    const PREFIX: &str = "keychain:";

    pub fn seal(secret: &str) -> Option<String> {
        let id = uuid::Uuid::new_v4().to_string();
        #[cfg(target_os = "macos")]
        let ok = Command::new("security")
            .args(["add-generic-password", "-U", "-a", "neru", "-s", &format!("neru.{id}"), "-w", secret])
            .status()
            .ok()?
            .success();
        #[cfg(not(target_os = "macos"))]
        let ok = {
            let mut child = Command::new("secret-tool")
                .args(["store", "--label=Neru API key", "service", "neru", "key", &id])
                .stdin(Stdio::piped())
                .spawn()
                .ok()?;
            child.stdin.take()?.write_all(secret.as_bytes()).ok()?;
            child.wait().ok()?.success()
        };
        ok.then(|| format!("{PREFIX}{id}"))
    }

    pub fn open(sealed: &str) -> Option<String> {
        let id = sealed.strip_prefix(PREFIX)?;
        #[cfg(target_os = "macos")]
        let output = Command::new("security")
            .args(["find-generic-password", "-a", "neru", "-s", &format!("neru.{id}"), "-w"])
            .output()
            .ok()?;
        #[cfg(not(target_os = "macos"))]
        let output = Command::new("secret-tool")
            .args(["lookup", "service", "neru", "key", id])
            .output()
            .ok()?;
        let _ = Stdio::null();
        output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim_end_matches('\n').to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        assert_eq!(from_hex(&to_hex(&[0, 15, 255])).unwrap(), vec![0, 15, 255]);
        assert!(from_hex("abc").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn keys_round_trip_through_dpapi() {
        let sealed = secret::seal("sk-test-123").unwrap();
        assert!(!sealed.contains("sk-test"));
        assert_eq!(secret::open(&sealed).unwrap(), "sk-test-123");
    }
}
