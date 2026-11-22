use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::paths;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct Model {
    pub id: &'static str,
    pub label: &'static str,
    pub file: &'static str,
    pub size_bytes: u64,
    pub sha256: &'static str,
    pub english_only: bool,
    pub description: &'static str,
}

const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// Pinned whisper.cpp GGML models; every download is checked against its SHA-256.
pub const CATALOG: &[Model] = &[
    Model {
        id: "base.en",
        label: "Whisper Base (English)",
        file: "ggml-base.en.bin",
        size_bytes: 147_964_211,
        sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        english_only: true,
        description: "Recommended. Fast and accurate for English.",
    },
    Model {
        id: "tiny.en",
        label: "Whisper Tiny (English)",
        file: "ggml-tiny.en.bin",
        size_bytes: 77_704_715,
        sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
        english_only: true,
        description: "Smallest and fastest; less accurate.",
    },
    Model {
        id: "small.en",
        label: "Whisper Small (English)",
        file: "ggml-small.en.bin",
        size_bytes: 487_614_201,
        sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
        english_only: true,
        description: "More accurate English; slower on CPU.",
    },
    Model {
        id: "small",
        label: "Whisper Small (multilingual)",
        file: "ggml-small.bin",
        size_bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        english_only: false,
        description: "99 languages including Gujarati and Hindi.",
    },
    Model {
        id: "large-v3-turbo-q5_0",
        label: "Whisper Large v3 Turbo (multilingual, quantized)",
        file: "ggml-large-v3-turbo-q5_0.bin",
        size_bytes: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        english_only: false,
        description: "Best multilingual quality; needs a fast machine.",
    },
];

pub const DEFAULT_MODEL: &str = "base.en";

pub fn find(id: &str) -> Result<&'static Model> {
    CATALOG
        .iter()
        .find(|m| m.id == id)
        .with_context(|| format!("unknown model `{id}`"))
}

impl Model {
    pub fn url(&self) -> String {
        format!("{BASE_URL}/{}", self.file)
    }

    pub fn path(&self) -> Result<PathBuf> {
        Ok(paths::models_dir()?.join(self.file))
    }

    /// Installed means present with the catalog's size. The hash is checked on download.
    pub fn is_installed(&self) -> bool {
        self.path()
            .ok()
            .and_then(|p| std::fs::metadata(p).ok())
            .is_some_and(|m| m.len() == self.size_bytes)
    }

    /// Downloads into a temporary file, verifies the SHA-256, then renames it into place.
    /// `progress(downloaded, total)` is called as bytes arrive.
    pub fn download(&self, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf> {
        let dest = self.path()?;
        if self.is_installed() {
            progress(self.size_bytes, self.size_bytes);
            return Ok(dest);
        }
        let partial = dest.with_extension("partial");
        let response = ureq::get(&self.url())
            .call()
            .with_context(|| format!("downloading {}", self.url()))?;
        let total = response.body().content_length().unwrap_or(self.size_bytes);
        let mut reader = response.into_body().into_reader();
        let mut out = File::create(&partial)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut done = 0u64;
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
            done += n as u64;
            progress(done, total);
        }
        out.sync_all()?;
        drop(out);
        let digest = hex(&hasher.finalize());
        if digest != self.sha256 {
            let _ = std::fs::remove_file(&partial);
            bail!("{} failed its checksum (got {digest})", self.file);
        }
        std::fs::rename(&partial, &dest)?;
        Ok(dest)
    }

    pub fn verify(&self) -> Result<bool> {
        let mut reader = BufReader::new(File::open(self.path()?)?);
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hex(&hasher.finalize()) == self.sha256)
    }

    pub fn remove(&self) -> Result<()> {
        let path = self.path()?;
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_well_formed() {
        assert!(find(DEFAULT_MODEL).is_ok());
        for m in CATALOG {
            assert_eq!(m.sha256.len(), 64, "{}", m.id);
            assert!(m.file.starts_with("ggml-") && m.file.ends_with(".bin"));
            assert_eq!(m.english_only, m.id.contains(".en"));
        }
    }
}
