//! Documents the user attaches from anywhere on their machine (outside the open project).
//! Text files are read here; images are sent to the model as data URLs; PDF and Word files are
//! handed to the window as bytes, where PDF.js and a small .docx reader extract their text.

use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

const MAX_TEXT: u64 = 1_000_000;
const MAX_IMAGE: u64 = 5_000_000;
const MAX_BINARY_DOCUMENT: u64 = 25_000_000;

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub name: String,
    pub path: String,
    pub size: u64,
    /// "text", "image", "pdf" or "docx".
    pub kind: String,
}

/// A document as sent with a message: text formats are read from `path`; PDF and Word files
/// carry the text the window extracted.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DocumentInput {
    pub path: String,
    #[serde(default)]
    pub text: Option<String>,
    /// Display name when `path` is not a file (an element picked in the preview).
    #[serde(default)]
    pub name: Option<String>,
    /// "element" for a page element picked in the in-app browser; otherwise a document.
    #[serde(default)]
    pub kind: Option<String>,
}

/// An image as sent with a message.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ImageInput {
    pub name: String,
    pub data_url: String,
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

pub fn image_mime(extension: &str) -> Option<&'static str> {
    match extension {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

fn kind(path: &Path) -> &'static str {
    let ext = extension(path);
    if image_mime(&ext).is_some() {
        "image"
    } else if ext == "pdf" {
        "pdf"
    } else if matches!(ext.as_str(), "docx" | "pptx" | "xlsx" | "odt" | "odp" | "ods" | "rtf") {
        // Office files are read in the window; the kind is the extension.
        match ext.as_str() {
            "docx" => "docx",
            "pptx" => "pptx",
            "xlsx" => "xlsx",
            "odt" => "odt",
            "odp" => "odp",
            "ods" => "ods",
            _ => "rtf",
        }
    } else {
        "text"
    }
}

fn checked(path: &str, limit: u64) -> Result<(&Path, String, u64), String> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err("Choose the document with the file picker".into());
    }
    let name = file_name(path);
    let metadata = fs::metadata(path).map_err(|e| format!("{name}: {e}"))?;
    if !metadata.is_file() {
        return Err(format!("{name} is not a file"));
    }
    if metadata.len() > limit {
        return Err(format!(
            "{name} is larger than the {} MB limit for this kind of file",
            limit / 1_000_000
        ));
    }
    Ok((path, name, metadata.len()))
}

/// Reads a user-chosen text document, returning its file name and contents.
pub fn read_document(path: &str) -> Result<(String, String), String> {
    let (path, name, _) = checked(path, MAX_TEXT)?;
    let bytes = fs::read(path).map_err(|e| format!("{name}: {e}"))?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    if bytes.iter().take(8192).any(|byte| *byte == 0) {
        return Err(format!(
            "{name} is not a text document. Attach text, Markdown, CSV, JSON, code, PDF, Word or image files."
        ));
    }
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| {
        format!("{name} is not UTF-8 text. Save it as UTF-8 or attach a text export.")
    })?;
    Ok((name, text))
}

/// Validates documents picked in the native dialog so the composer can show them before sending.
#[tauri::command]
pub fn inspect_documents(paths: Vec<String>) -> Result<Vec<Document>, String> {
    paths
        .into_iter()
        .map(|path| {
            let kind = kind(Path::new(&path));
            let ext = extension(Path::new(&path));
            if matches!(ext.as_str(), "doc" | "ppt" | "xls") {
                return Err(format!("{} is an old Office format Neru cannot read. Save it as .{}x and attach that instead.", Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), ext));
            }
            let (name, size) = match kind {
                "text" => {
                    let (name, text) = read_document(&path)?;
                    (name, text.len() as u64)
                }
                "image" => {
                    let (_, name, size) = checked(&path, MAX_IMAGE)?;
                    (name, size)
                }
                _ => {
                    let (_, name, size) = checked(&path, MAX_BINARY_DOCUMENT)?;
                    (name, size)
                }
            };
            Ok(Document {
                name,
                path,
                size,
                kind: kind.into(),
            })
        })
        .collect()
}

/// The raw bytes of a PDF or Word document, for text extraction in the window.
#[tauri::command]
pub fn document_bytes(path: String) -> Result<tauri::ipc::Response, String> {
    if !matches!(kind(Path::new(&path)), "pdf" | "docx" | "pptx" | "xlsx" | "odt" | "odp" | "ods" | "rtf") {
        return Err("Only PDF and Office documents are read this way".into());
    }
    let (path, name, _) = checked(&path, MAX_BINARY_DOCUMENT)?;
    let bytes = fs::read(path).map_err(|e| format!("{name}: {e}"))?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// An image file as a `data:` URL, ready to send to a vision model or preview.
#[tauri::command]
pub fn read_image(path: String) -> Result<String, String> {
    let (path, name, _) = checked(&path, MAX_IMAGE)?;
    let mime = image_mime(&extension(path)).ok_or(format!("{name} is not a PNG, JPEG, GIF or WebP image"))?;
    let bytes = fs::read(path).map_err(|e| format!("{name}: {e}"))?;
    Ok(format!("data:{mime};base64,{}", base64(&bytes)))
}

/// Checks an image sent with a message: a base64 PNG, JPEG, GIF or WebP data URL within the limit.
pub fn validate_image(image: &ImageInput) -> Result<(), String> {
    let (mime, data) = image
        .data_url
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(";base64,"))
        .ok_or(format!("{} is not an embedded image", image.name))?;
    if !matches!(mime, "image/png" | "image/jpeg" | "image/gif" | "image/webp") {
        return Err(format!("{} is not a PNG, JPEG, GIF or WebP image", image.name));
    }
    if data.len() as u64 > MAX_IMAGE * 4 / 3 + 4 {
        return Err(format!("{} is larger than the 5 MB image limit", image.name));
    }
    if !data
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return Err(format!("{} is not valid base64", image.name));
    }
    Ok(())
}

pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

/// The reverse of `base64`, for images pasted into a message.
pub fn unbase64(text: &str) -> Result<Vec<u8>, String> {
    let value = |c: u8| -> Result<u32, String> {
        Ok(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return Err(format!("{:?} is not base64", c as char)),
        } as u32)
    };
    let clean: Vec<u8> = text.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (index, c) in chunk.iter().enumerate() {
            n |= value(*c)? << (18 - 6 * index);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        for bytes in [&b""[..], b"a", b"ab", b"abc", &[0, 255, 16, 32, 105]] {
            assert_eq!(unbase64(&base64(bytes)).unwrap(), bytes);
        }
        assert!(unbase64("not*base64").is_err());
    }

    #[test]
    fn reads_text_and_refuses_binary() {
        let dir = std::env::temp_dir().join(format!("neru-docs-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let text = dir.join("notes.md");
        fs::write(&text, "\u{feff}# Notes\nhello").unwrap();
        let binary = dir.join("blob.bin");
        fs::write(&binary, [0x89, b'P', b'N', b'G', 0, 0, 0]).unwrap();
        let image = dir.join("shot.png");
        fs::write(&image, [0x89, b'P', b'N', b'G']).unwrap();

        let (name, body) = read_document(text.to_str().unwrap()).unwrap();
        assert_eq!(name, "notes.md");
        assert_eq!(body, "# Notes\nhello");
        assert!(read_document(binary.to_str().unwrap()).unwrap_err().contains("not a text document"));
        assert!(read_document("relative.txt").is_err());
        let docs = inspect_documents(vec![image.to_string_lossy().into()]).unwrap();
        assert_eq!(docs[0].kind, "image");
        assert_eq!(read_image(image.to_string_lossy().into()).unwrap(), "data:image/png;base64,iVBORw==");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        let ok = ImageInput { name: "a.png".into(), data_url: "data:image/png;base64,iVBORw==".into() };
        assert!(validate_image(&ok).is_ok());
        let bad = ImageInput { name: "a.svg".into(), data_url: "data:image/svg+xml;base64,PHN2Zz4=".into() };
        assert!(validate_image(&bad).is_err());
    }
}
