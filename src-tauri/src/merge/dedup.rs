//! Exact row membership with a bounded in-memory key store and temporary disk spill.
use super::CellValue;
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::HashSet;
use tempfile::TempDir;

const MEMORY_BUDGET: usize = 64 * 1024 * 1024;

pub(super) struct Deduplicator {
    memory: HashSet<Box<[u8]>>,
    key_bytes: usize,
    budget: usize,
    disk: Option<DiskKeys>,
}
struct DiskKeys {
    // Drop the connection before removing its directory, including on Windows.
    connection: Connection,
    _directory: TempDir,
}
impl Default for Deduplicator {
    fn default() -> Self {
        Self::with_budget(MEMORY_BUDGET)
    }
}
impl Deduplicator {
    fn with_budget(budget: usize) -> Self {
        Self {
            memory: HashSet::new(),
            key_bytes: 0,
            budget,
            disk: None,
        }
    }
    pub(super) fn insert(&mut self, row: &[CellValue]) -> Result<bool> {
        let mut key = Vec::new();
        for value in row {
            let text = value.copy_text(false).unwrap_or_default();
            key.extend_from_slice(&(text.len() as u64).to_le_bytes());
            key.extend_from_slice(text.as_bytes());
        }
        if self.disk.is_none() {
            if self.memory.contains(key.as_slice()) {
                return Ok(false);
            }
            // Allow for a hash table resize and allocator/control bytes, not only text.
            let projected = self.key_bytes.saturating_add(key.len()).saturating_add(
                self.memory
                    .capacity()
                    .max(self.memory.len() + 1)
                    .saturating_mul(64),
            );
            if projected <= self.budget {
                self.key_bytes += key.len();
                self.memory.insert(key.into_boxed_slice());
                return Ok(true);
            }
            self.spill()?;
        }
        self.disk.as_ref().expect("spill initialized").insert(&key)
    }
    fn spill(&mut self) -> Result<()> {
        let directory = tempfile::Builder::new()
            .prefix("sheetforge-dedup-")
            .tempdir()
            .context("无法创建去重临时目录")?;
        let connection = Connection::open(directory.path().join("keys.sqlite"))
            .context("无法创建去重临时索引")?;
        // This database is disposable scratch data. Never apply these settings to user data.
        connection.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA cache_size=-8192; PRAGMA temp_store=FILE; PRAGMA locking_mode=EXCLUSIVE; CREATE TABLE seen (k BLOB PRIMARY KEY) WITHOUT ROWID; BEGIN;")?;
        let disk = DiskKeys {
            connection,
            _directory: directory,
        };
        for key in &self.memory {
            disk.insert(key)?;
        }
        self.memory = HashSet::new();
        self.key_bytes = 0;
        self.disk = Some(disk);
        Ok(())
    }
}
impl DiskKeys {
    fn insert(&self, key: &[u8]) -> Result<bool> {
        Ok(self
            .connection
            .prepare_cached("INSERT OR IGNORE INTO seen(k) VALUES (?1)")?
            .execute([key])
            .context("去重临时索引写入失败，请检查临时磁盘空间")?
            == 1)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn text(value: &str) -> CellValue {
        CellValue::Text(value.into())
    }
    #[test]
    fn spill_retains_exact_equality_order_and_cleans_up() {
        let mut keys = Deduplicator::with_budget(256);
        let first = vec![text("甲\0🙂"), text("ab"), text("c")];
        assert!(keys.insert(&first).unwrap());
        assert!(!keys.insert(&first).unwrap());
        for index in 0..1000 {
            assert!(keys.insert(&[text(&index.to_string())]).unwrap());
        }
        let directory = keys.disk.as_ref().unwrap()._directory.path().to_owned();
        assert!(!keys.insert(&first).unwrap());
        assert!(keys
            .insert(&[text("甲\0🙂"), text("a"), text("bc")])
            .unwrap());
        // Keep the existing canonical string equality, including empty/missing cells.
        assert!(keys
            .insert(&[CellValue::Empty, CellValue::Integer(1)])
            .unwrap());
        assert!(!keys.insert(&[text(""), text("1")]).unwrap());
        assert!(keys.insert(&[text("")]).unwrap());
        drop(keys);
        assert!(!directory.exists());
    }
}
