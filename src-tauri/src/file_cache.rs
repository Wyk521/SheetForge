//! Small revision-keyed caches. Never hold a cache lock while reading a source.
use std::{collections::HashMap, path::Path, time::SystemTime};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FileVersion {
    size: u64,
    modified: SystemTime,
    created: Option<SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}
impl FileVersion {
    pub(crate) fn read(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Some(Self {
            size: metadata.len(),
            modified: metadata.modified().ok()?,
            created: metadata.created().ok(),
            #[cfg(unix)]
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        })
    }
}

pub(crate) struct Cache<K, V> {
    entries: HashMap<K, (V, usize, u64)>,
    bytes: usize,
    clock: u64,
    max_entries: usize,
    max_bytes: usize,
}
impl<K: Eq + std::hash::Hash + Clone, V: Clone> Cache<K, V> {
    pub(crate) fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            bytes: 0,
            clock: 0,
            max_entries,
            max_bytes,
        }
    }
    pub(crate) fn get(&mut self, key: &K) -> Option<V> {
        self.clock = self.clock.wrapping_add(1);
        let entry = self.entries.get_mut(key)?;
        entry.2 = self.clock;
        Some(entry.0.clone())
    }
    pub(crate) fn insert(&mut self, key: K, value: V, bytes: usize) {
        if let Some((_, old, _)) = self.entries.remove(&key) {
            self.bytes -= old;
        }
        if bytes > self.max_bytes || self.max_entries == 0 {
            return;
        }
        while self.entries.len() >= self.max_entries
            || self.bytes.saturating_add(bytes) > self.max_bytes
        {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.2)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some((_, size, _)) = self.entries.remove(&oldest) {
                self.bytes -= size;
            }
        }
        self.clock = self.clock.wrapping_add(1);
        self.bytes += bytes;
        self.entries.insert(key, (value, bytes, self.clock));
    }
    pub(crate) fn retain(&mut self, mut predicate: impl FnMut(&K) -> bool) {
        self.entries.retain(|key, (_, bytes, _)| {
            if predicate(key) {
                true
            } else {
                self.bytes -= *bytes;
                false
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evicts_by_bytes_and_recency_and_rejects_oversize() {
        let mut cache = Cache::new(3, 10);
        cache.insert(1, "one", 4);
        cache.insert(2, "two", 4);
        assert_eq!(cache.get(&1), Some("one"));
        cache.insert(3, "three", 4);
        assert_eq!(cache.get(&2), None);
        assert_eq!(cache.get(&1), Some("one"));
        cache.insert(1, "oversize", 11);
        assert_eq!(cache.get(&1), None);
        cache.retain(|_| false);
        assert_eq!(cache.bytes, 0);
    }
    #[test]
    fn same_path_replacement_and_deletion_change_version() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("data");
        std::fs::write(&path, "before").unwrap();
        let first = FileVersion::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(FileVersion::read(&path), None);
        std::fs::write(&path, "after replacement").unwrap();
        assert_ne!(FileVersion::read(&path).unwrap(), first);
    }
}
