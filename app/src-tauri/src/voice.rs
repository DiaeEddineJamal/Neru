use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::{AppState, ProviderConfig, providers};

const MAX_AUDIO_BYTES: usize = 24 * 1024 * 1024;

#[derive(Clone, Default)]
pub struct VoiceConfig {
    /// Empty means "use the chat provider's endpoint and key".
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceView {
    pub base_url: String,
    pub model: String,
    pub has_key: bool,
    pub uses_chat_provider: bool,
}

fn view(config: &VoiceConfig) -> VoiceView {
    VoiceView {
        base_url: config.base_url.clone(),
        model: if config.model.is_empty() {
            "whisper-1".into()
        } else {
            config.model.clone()
        },
        has_key: !config.api_key.is_empty(),
        uses_chat_provider: config.base_url.is_empty(),
    }
}

#[tauri::command]
pub fn voice_status(state: State<'_, AppState>) -> Result<VoiceView, String> {
    let voice = state.voice.lock().map_err(|e| e.to_string())?;
    Ok(view(&voice))
}

#[tauri::command]
pub fn configure_voice(
    base_url: String,
    api_key: String,
    model: String,
    state: State<'_, AppState>,
) -> Result<VoiceView, String> {
    let base_url = base_url.trim().trim_end_matches('/').to_string();
    if !base_url.is_empty() {
        let url = reqwest::Url::parse(&base_url).map_err(|e| e.to_string())?;
        if url.scheme() != "https" && !(url.scheme() == "http" && providers::is_local(&url)) {
            return Err("Use HTTPS, or HTTP on localhost".into());
        }
        if url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
        {
            return Err("Enter a base URL without credentials or query".into());
        }
    }
    let model = model.trim();
    if model.is_empty() {
        return Err("Enter a transcription model".into());
    }
    let mut voice = state.voice.lock().map_err(|e| e.to_string())?;
    let keep_key = api_key.trim().is_empty() && voice.base_url == base_url;
    voice.api_key = if keep_key {
        voice.api_key.clone()
    } else {
        api_key.trim().to_string()
    };
    voice.base_url = base_url;
    voice.model = model.to_string();
    let result = view(&voice);
    drop(voice);
    crate::settings::save(&state)?;
    Ok(result)
}

fn extension(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or("").trim() {
        "audio/ogg" => "ogg",
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => "m4a",
        "audio/mpeg" => "mp3",
        "audio/wav" | "audio/x-wav" => "wav",
        _ => "webm",
    }
}

fn multipart(boundary: &str, model: &str, audio: &[u8], mime: &str) -> Vec<u8> {
    let mut body = Vec::with_capacity(audio.len() + 512);
    let field = |body: &mut Vec<u8>, name: &str, value: &str| {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    };
    field(&mut body, "model", model);
    field(&mut body, "response_format", "json");
    let content_type = mime.split(';').next().unwrap_or("audio/webm").trim();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.{}\"\r\nContent-Type: {content_type}\r\n\r\n",
            extension(mime)
        )
        .as_bytes(),
    );
    body.extend_from_slice(audio);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

#[tauri::command]
pub async fn transcribe_audio(
    audio: Vec<u8>,
    mime_type: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    if audio.is_empty() {
        return Err("No audio was recorded".into());
    }
    if audio.len() > MAX_AUDIO_BYTES {
        return Err("Recording is too long to transcribe".into());
    }
    let voice = state.voice.lock().map_err(|e| e.to_string())?.clone();
    let chat = state.provider.lock().map_err(|e| e.to_string())?.clone();
    let model = if voice.model.is_empty() {
        "whisper-1".to_string()
    } else {
        voice.model.clone()
    };
    let config = if voice.base_url.is_empty() {
        if chat.api_format == providers::ANTHROPIC {
            return Err("Anthropic has no speech-to-text API. Set a transcription endpoint in Settings → Voice.".into());
        }
        ProviderConfig { model, ..chat }
    } else {
        ProviderConfig {
            provider_id: "voice".into(),
            api_format: providers::CHAT.into(),
            base_url: voice.base_url.clone(),
            api_key: voice.api_key.clone(),
            model,
        }
    };
    let boundary = format!("neru-{}", uuid::Uuid::new_v4().simple());
    let body = multipart(&boundary, &config.model, &audio, &mime_type);
    let request = reqwest::Client::new()
        .post(format!("{}/audio/transcriptions", config.base_url))
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .timeout(Duration::from_secs(60));
    let response = providers::authorize(request, &config)
        .send()
        .await
        .map_err(|e| format!("Transcription request failed: {e}"))?;
    let body: Value = providers::read_response(response).await?;
    body["text"]
        .as_str()
        .map(|text| text.trim().to_string())
        .ok_or_else(|| "Transcription service returned no text".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_openai_style_multipart() {
        let body = String::from_utf8(multipart(
            "b",
            "whisper-1",
            b"AUDIO",
            "audio/webm;codecs=opus",
        ))
        .unwrap();
        assert!(body.contains("name=\"model\"\r\n\r\nwhisper-1\r\n"));
        assert!(body.contains(
            "filename=\"speech.webm\"\r\nContent-Type: audio/webm\r\n\r\nAUDIO\r\n--b--"
        ));
    }
}
