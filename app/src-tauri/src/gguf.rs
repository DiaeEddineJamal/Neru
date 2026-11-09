//! Reads the metadata at the start of a GGUF file (llama.cpp's model format): the architecture, the
//! number of layers, the attention shape and the trained context length. The built-in engine uses it
//! to size the context and the layers it puts on the graphics card from what the model really needs,
//! instead of guessing from the file name. Only the header is read; the tensors are skipped.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GgufInfo {
    pub architecture: String,
    pub name: String,
    pub layers: u32,
    pub context_length: u32,
    pub embedding_length: u32,
    pub heads: u32,
    /// Key/value heads (fewer than `heads` with grouped-query attention). The largest when it varies by layer.
    pub kv_heads: u32,
    pub key_length: u32,
    pub value_length: u32,
    /// Layers that attend only to a window of recent tokens keep a smaller cache (Gemma 3, gpt-oss).
    pub sliding_window: u32,
    pub experts: u32,
    /// Whether the chat template is in the file (llama-server's `--jinja` uses it).
    pub has_template: bool,
}

impl GgufInfo {
    /// Bytes the key/value cache takes for each token of context at `bytes_per_value` (2 for f16,
    /// about 1.06 for q8_0).
    pub fn kv_bytes_per_token(&self, bytes_per_value: f32) -> f32 {
        let heads = if self.kv_heads > 0 { self.kv_heads } else { self.heads.max(1) };
        let head = if self.key_length > 0 { self.key_length } else { self.embedding_length / self.heads.max(1) };
        let value = if self.value_length > 0 { self.value_length } else { head };
        self.layers as f32 * heads as f32 * (head + value) as f32 * bytes_per_value
    }
}

#[derive(Debug)]
enum Meta {
    Int(i64),
    Float(f64),
    Text(String),
    Ints(Vec<i64>),
    Other,
}

struct Reader<R> {
    inner: R,
}

impl<R: Read + Seek> Reader<R> {
    fn bytes<const N: usize>(&mut self) -> std::io::Result<[u8; N]> {
        let mut buffer = [0u8; N];
        self.inner.read_exact(&mut buffer)?;
        Ok(buffer)
    }
    fn u32(&mut self) -> std::io::Result<u32> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }
    fn u64(&mut self) -> std::io::Result<u64> {
        Ok(u64::from_le_bytes(self.bytes()?))
    }
    fn string(&mut self) -> std::io::Result<String> {
        let length = self.u64()?;
        if length > 1 << 24 {
            return Err(std::io::Error::other("a metadata string is too long"));
        }
        let mut buffer = vec![0u8; length as usize];
        self.inner.read_exact(&mut buffer)?;
        Ok(String::from_utf8_lossy(&buffer).into_owned())
    }
    fn skip(&mut self, bytes: u64) -> std::io::Result<()> {
        self.inner.seek(SeekFrom::Current(bytes as i64)).map(|_| ())
    }
    /// A scalar of type `kind`, as an integer or a float; strings are read, arrays handled by the caller.
    fn scalar(&mut self, kind: u32) -> std::io::Result<Meta> {
        Ok(match kind {
            0 => Meta::Int(self.bytes::<1>()?[0] as i64),
            1 => Meta::Int(self.bytes::<1>()?[0] as i8 as i64),
            2 => Meta::Int(u16::from_le_bytes(self.bytes()?) as i64),
            3 => Meta::Int(i16::from_le_bytes(self.bytes()?) as i64),
            4 => Meta::Int(self.u32()? as i64),
            5 => Meta::Int(i32::from_le_bytes(self.bytes()?) as i64),
            6 => Meta::Float(f32::from_le_bytes(self.bytes()?) as f64),
            7 => Meta::Int(self.bytes::<1>()?[0] as i64),
            8 => Meta::Text(self.string()?),
            10 => Meta::Int(self.u64()? as i64),
            11 => Meta::Int(i64::from_le_bytes(self.bytes()?)),
            12 => Meta::Float(f64::from_le_bytes(self.bytes()?)),
            _ => return Err(std::io::Error::other(format!("unknown metadata type {kind}"))),
        })
    }
    fn value(&mut self, kind: u32) -> std::io::Result<Meta> {
        if kind != 9 {
            return self.scalar(kind);
        }
        let (item, count) = (self.u32()?, self.u64()?);
        let width = match item {
            0 | 1 | 7 => Some(1),
            2 | 3 => Some(2),
            4..=6 => Some(4),
            10..=12 => Some(8),
            _ => None,
        };
        // Integer arrays are kept when short (per-layer head counts); the vocabulary is skipped.
        if matches!(item, 0..=5 | 10 | 11) && count <= 1024 {
            let mut ints = Vec::with_capacity(count as usize);
            for _ in 0..count {
                if let Meta::Int(value) = self.scalar(item)? {
                    ints.push(value);
                }
            }
            return Ok(Meta::Ints(ints));
        }
        match width {
            Some(width) => self.skip(width * count)?,
            None if item == 8 => {
                for _ in 0..count {
                    let length = self.u64()?;
                    self.skip(length)?;
                }
            }
            None => return Err(std::io::Error::other("nested metadata arrays are not supported")),
        }
        Ok(Meta::Other)
    }
}

/// Reads the header of the GGUF file at `path`.
pub fn read(path: &Path) -> Result<GgufInfo, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    read_from(BufReader::with_capacity(1 << 16, file))
}

pub fn read_from<R: BufRead + Seek>(inner: R) -> Result<GgufInfo, String> {
    let mut reader = Reader { inner };
    let error = |e: std::io::Error| format!("not a readable GGUF file: {e}");
    if &reader.bytes::<4>().map_err(error)? != b"GGUF" {
        return Err("not a GGUF file".into());
    }
    let version = reader.u32().map_err(error)?;
    if version < 2 {
        return Err(format!("GGUF version {version} is too old"));
    }
    let _tensors = reader.u64().map_err(error)?;
    let count = reader.u64().map_err(error)?;
    let mut meta = HashMap::new();
    for _ in 0..count.min(100_000) {
        let key = reader.string().map_err(error)?;
        let kind = reader.u32().map_err(error)?;
        let value = reader.value(kind).map_err(error)?;
        meta.insert(key, value);
    }
    Ok(info_from(&meta))
}

fn info_from(meta: &HashMap<String, Meta>) -> GgufInfo {
    let text = |key: &str| match meta.get(key) {
        Some(Meta::Text(text)) => text.clone(),
        _ => String::new(),
    };
    let architecture = text("general.architecture");
    let int = |suffix: &str| -> u32 {
        match meta.get(&format!("{architecture}.{suffix}")) {
            Some(Meta::Int(value)) => (*value).max(0) as u32,
            Some(Meta::Float(value)) => value.max(0.0) as u32,
            Some(Meta::Ints(values)) => values.iter().copied().max().unwrap_or(0).max(0) as u32,
            _ => 0,
        }
    };
    GgufInfo {
        name: text("general.name"),
        layers: int("block_count"),
        context_length: int("context_length"),
        embedding_length: int("embedding_length"),
        heads: int("attention.head_count"),
        kv_heads: int("attention.head_count_kv"),
        key_length: int("attention.key_length"),
        value_length: int("attention.value_length"),
        sliding_window: int("attention.sliding_window"),
        experts: int("expert_count"),
        has_template: matches!(meta.get("tokenizer.chat_template"), Some(Meta::Text(_))),
        architecture,
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn string(out: &mut Vec<u8>, text: &str) {
        out.extend((text.len() as u64).to_le_bytes());
        out.extend(text.as_bytes());
    }

    /// A tiny GGUF header shaped like a Qwen3 4B, with a vocabulary to skip.
    pub fn sample() -> Vec<u8> {
        let mut out = b"GGUF".to_vec();
        out.extend(3u32.to_le_bytes());
        out.extend(0u64.to_le_bytes());
        let entries: Vec<(&str, Box<dyn Fn(&mut Vec<u8>)>)> = vec![
            ("general.architecture", Box::new(|o: &mut Vec<u8>| { o.extend(8u32.to_le_bytes()); string(o, "qwen3") })),
            ("general.name", Box::new(|o: &mut Vec<u8>| { o.extend(8u32.to_le_bytes()); string(o, "Qwen3 4B") })),
            ("tokenizer.ggml.tokens", Box::new(|o: &mut Vec<u8>| { o.extend(9u32.to_le_bytes()); o.extend(8u32.to_le_bytes()); o.extend(3u64.to_le_bytes()); for t in ["a", "bb", "<|im_start|>"] { string(o, t) } })),
            ("tokenizer.ggml.scores", Box::new(|o: &mut Vec<u8>| { o.extend(9u32.to_le_bytes()); o.extend(6u32.to_le_bytes()); o.extend(3u64.to_le_bytes()); o.extend([0u8; 12]) })),
            ("qwen3.block_count", Box::new(|o: &mut Vec<u8>| { o.extend(4u32.to_le_bytes()); o.extend(36u32.to_le_bytes()) })),
            ("qwen3.context_length", Box::new(|o: &mut Vec<u8>| { o.extend(4u32.to_le_bytes()); o.extend(262_144u32.to_le_bytes()) })),
            ("qwen3.embedding_length", Box::new(|o: &mut Vec<u8>| { o.extend(4u32.to_le_bytes()); o.extend(2560u32.to_le_bytes()) })),
            ("qwen3.attention.head_count", Box::new(|o: &mut Vec<u8>| { o.extend(4u32.to_le_bytes()); o.extend(32u32.to_le_bytes()) })),
            ("qwen3.attention.head_count_kv", Box::new(|o: &mut Vec<u8>| { o.extend(4u32.to_le_bytes()); o.extend(8u32.to_le_bytes()) })),
            ("qwen3.attention.key_length", Box::new(|o: &mut Vec<u8>| { o.extend(4u32.to_le_bytes()); o.extend(128u32.to_le_bytes()) })),
            ("qwen3.attention.value_length", Box::new(|o: &mut Vec<u8>| { o.extend(4u32.to_le_bytes()); o.extend(128u32.to_le_bytes()) })),
            ("tokenizer.chat_template", Box::new(|o: &mut Vec<u8>| { o.extend(8u32.to_le_bytes()); string(o, "{{ messages }}") })),
        ];
        out.extend((entries.len() as u64).to_le_bytes());
        for (key, write) in &entries {
            string(&mut out, key);
            write(&mut out);
        }
        out
    }

    #[test]
    fn reads_the_attention_shape_and_skips_the_vocabulary() {
        let info = read_from(std::io::Cursor::new(sample())).unwrap();
        assert_eq!((info.architecture.as_str(), info.layers, info.kv_heads, info.key_length, info.context_length), ("qwen3", 36, 8, 128, 262_144));
        assert!(info.has_template);
        // 36 layers × 8 heads × (128 + 128) × 2 bytes = 147,456 bytes a token at f16.
        assert_eq!(info.kv_bytes_per_token(2.0), 147_456.0);
    }

    #[test]
    fn other_files_are_refused() {
        assert!(read_from(std::io::Cursor::new(b"PK\x03\x04rest".to_vec())).unwrap_err().contains("not a GGUF"));
        assert!(read_from(std::io::Cursor::new(b"GGUF".to_vec())).is_err());
    }
}
