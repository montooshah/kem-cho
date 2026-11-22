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
    /// Languages the model can transcribe; empty means any Whisper language.
    pub languages: &'static [&'static str],
    pub description: &'static str,
    /// Download location without the file name.
    pub source: &'static str,
    /// Fine-tunes trained without timestamp tokens decode badly with them.
    pub no_timestamps: bool,
}

const WHISPER_CPP: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";
/// IIT Madras Speech Lab fine-tunes (Apache 2.0), converted to GGML by Ukta.
/// Oriserve's Hinglish fine-tune of Large v3 Turbo (Apache 2.0), converted by Marquestra.
const HINGLISH: &str =
    "https://huggingface.co/Marquestra/Whisper-Hindi2Hinglish-Apex-GGML/resolve/main";
const INDIC_WHISPER: &str = "https://huggingface.co/ukta-app/indic-whisper-ggml/resolve/main";

/// Pinned whisper.cpp GGML models; every download is checked against its SHA-256.
pub const CATALOG: &[Model] = &[
    Model {
        id: "base.en",
        label: "Whisper Base (English)",
        file: "ggml-base.en.bin",
        size_bytes: 147_964_211,
        sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        languages: &["en"],
        description: "Recommended. Fast and accurate for English.",
        source: WHISPER_CPP,
        no_timestamps: false,
    },
    Model {
        id: "tiny.en",
        label: "Whisper Tiny (English)",
        file: "ggml-tiny.en.bin",
        size_bytes: 77_704_715,
        sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
        languages: &["en"],
        description: "Smallest and fastest; less accurate.",
        source: WHISPER_CPP,
        no_timestamps: false,
    },
    Model {
        id: "small.en",
        label: "Whisper Small (English)",
        file: "ggml-small.en.bin",
        size_bytes: 487_614_201,
        sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
        languages: &["en"],
        description: "More accurate English; slower on CPU.",
        source: WHISPER_CPP,
        no_timestamps: false,
    },
    Model {
        id: "small",
        label: "Whisper Small (multilingual)",
        file: "ggml-small.bin",
        size_bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        languages: &[],
        description: "99 languages including Gujarati and Hindi.",
        source: WHISPER_CPP,
        no_timestamps: false,
    },
    Model {
        id: "large-v3-turbo-q5_0",
        label: "Whisper Large v3 Turbo (multilingual, quantized)",
        file: "ggml-large-v3-turbo-q5_0.bin",
        size_bytes: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        languages: &[],
        description: "Best multilingual quality; needs a fast machine.",
        source: WHISPER_CPP,
        no_timestamps: false,
    },
    Model {
        id: "hi-small",
        label: "Whisper Small Hindi (IIT Madras)",
        file: "ggml-hi-small.bin",
        size_bytes: 190_085_487,
        sha256: "6813fed7ffa6c3fa14490c1f1788d2d8b6e3b7badf59a2f75fb4c5c21cf00f3f",
        languages: &["hi"],
        description: "Recommended for Hindi. Fine-tuned on Indian speech; Hindi only.",
        source: INDIC_WHISPER,
        no_timestamps: true,
    },
    Model {
        id: "gu-small",
        label: "Whisper Small Gujarati (IIT Madras)",
        file: "ggml-gu-small.bin",
        size_bytes: 190_085_487,
        sha256: "e756347b8fb573b88a5625ad98dac9075746549fdf407260d321d077a73b094a",
        languages: &["gu"],
        description: "Recommended for Gujarati. Fine-tuned on Indian speech; Gujarati only.",
        source: INDIC_WHISPER,
        no_timestamps: true,
    },
    Model {
        id: "hinglish",
        label: "Whisper Hinglish (Oriserve Apex)",
        file: "ggml-apex-hinglish-q5_0.bin",
        size_bytes: 574_041_195,
        sha256: "9d877151b15cec1feb9110cfbc0a3162cf377bcc0ab1935174226f461cf60f13",
        languages: &["hi"],
        description: "Recommended for Hindi + English. Writes Hindi in Roman letters and keeps English words in English.",
        source: HINGLISH,
        no_timestamps: false,
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
        format!("{}/{}", self.source, self.file)
    }

    pub fn supports(&self, whisper_language: &str) -> bool {
        self.languages.is_empty() || self.languages.contains(&whisper_language)
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

pub(crate) fn hex(bytes: &[u8]) -> String {
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
            assert_eq!(m.languages == ["en"], m.id.contains(".en"));
            assert!(m.source.starts_with("https://huggingface.co/"));
        }
        for l in crate::languages::LANGUAGES {
            assert!(
                find(l.default_model).unwrap().supports(l.whisper),
                "{}",
                l.id
            );
        }
    }
}
