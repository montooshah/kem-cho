use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;

fn project() -> Result<ProjectDirs> {
    ProjectDirs::from("app", "kemcho", "kem-cho").context("no home directory")
}

/// Root of everything Kem Cho stores. `KEMCHO_DATA_DIR` overrides it.
pub fn data_dir() -> Result<PathBuf> {
    let dir = match std::env::var_os("KEMCHO_DATA_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => project()?.data_dir().to_path_buf(),
    };
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

pub fn models_dir() -> Result<PathBuf> {
    let dir = data_dir()?.join("models");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn settings_file() -> Result<PathBuf> {
    Ok(data_dir()?.join("settings.json"))
}

pub fn history_file() -> Result<PathBuf> {
    Ok(data_dir()?.join("history.jsonl"))
}
