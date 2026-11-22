use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Milliseconds since the Unix epoch; unique per dictation.
    pub id: u64,
    pub text: String,
    pub raw: String,
    pub model: String,
    pub audio_ms: u64,
    pub transcribe_ms: u64,
}

/// Dictation history as JSON lines, kept for a fixed number of days.
pub struct History {
    path: PathBuf,
    /// Serializes appends with rewrites so neither loses the other's entries.
    lock: Mutex<()>,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

impl History {
    pub fn open() -> Result<Self> {
        Ok(Self::at(paths::history_file()?))
    }

    pub fn at(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            lock: Mutex::new(()),
        }
    }

    pub fn append(&self, entry: &Entry) -> Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{}", serde_json::to_string(entry)?)?;
        Ok(())
    }

    /// Newest first.
    pub fn list(&self) -> Result<Vec<Entry>> {
        let Ok(file) = std::fs::File::open(&self.path) else {
            return Ok(Vec::new());
        };
        let mut entries: Vec<Entry> = BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str(&line).ok())
            .collect();
        entries.reverse();
        Ok(entries)
    }

    pub fn search(&self, query: &str) -> Result<Vec<Entry>> {
        let q = query.to_lowercase();
        Ok(self
            .list()?
            .into_iter()
            .filter(|e| e.text.to_lowercase().contains(&q))
            .collect())
    }

    fn rewrite(&self, keep: impl Fn(&Entry) -> bool) -> Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut entries = self.list()?;
        entries.reverse();
        let tmp = self.path.with_extension("jsonl.tmp");
        let mut out = std::fs::File::create(&tmp)?;
        for e in entries.iter().filter(|e| keep(e)) {
            writeln!(out, "{}", serde_json::to_string(e)?)?;
        }
        out.sync_all()?;
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }

    pub fn delete(&self, id: u64) -> Result<()> {
        self.rewrite(|e| e.id != id)
    }

    pub fn clear(&self) -> Result<()> {
        self.rewrite(|_| false)
    }

    /// Drops entries older than `days`; zero keeps nothing.
    pub fn prune(&self, days: u32) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        let cutoff = now_ms().saturating_sub(days as u64 * 86_400_000);
        self.rewrite(|e| days > 0 && e.id >= cutoff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, text: &str) -> Entry {
        Entry {
            id,
            text: text.into(),
            raw: text.into(),
            model: "base.en".into(),
            audio_ms: 1000,
            transcribe_ms: 100,
        }
    }

    #[test]
    fn append_search_delete_prune() {
        let path =
            std::env::temp_dir().join(format!("kemcho-history-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let h = History::at(&path);
        h.append(&entry(1, "old one")).unwrap();
        h.append(&entry(now_ms(), "Hello Kem Cho")).unwrap();
        h.append(&entry(now_ms() + 1, "second")).unwrap();
        assert_eq!(h.list().unwrap()[0].text, "second");
        assert_eq!(h.search("kem").unwrap().len(), 1);
        h.prune(7).unwrap();
        assert_eq!(h.list().unwrap().len(), 2);
        let id = h.list().unwrap()[0].id;
        h.delete(id).unwrap();
        assert_eq!(h.list().unwrap().len(), 1);
        h.clear().unwrap();
        assert!(h.list().unwrap().is_empty());
        std::fs::remove_file(path).unwrap();
    }
}
