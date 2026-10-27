//! Optional private restart bodies. Disk data is never fresh authority.
//!
//! Readers must revalidate restored bytes with the origin. Captured write epochs
//! fence snapshots even when a mutation happened with no running daemon.

use crate::{coherence::Journal, platform, retention::Cache};
use prost::Message;
use std::{
    fs::{File, OpenOptions},
    io,
    path::PathBuf,
};

#[derive(Clone, PartialEq, Message)]
struct Record {
    #[prost(string, tag = "1")]
    key: String,
    #[prost(bytes = "vec", tag = "2")]
    body: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
struct Snapshot {
    #[prost(uint32, tag = "1")]
    version: u32,
    #[prost(string, tag = "2")]
    identity: String,
    #[prost(uint64, tag = "3")]
    epoch: u64,
    #[prost(message, repeated, tag = "4")]
    records: Vec<Record>,
}

#[derive(Clone, PartialEq, Message)]
struct Envelope {
    #[prost(bytes = "vec", tag = "1")]
    body: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    checksum: Vec<u8>,
}

pub struct Store {
    directory: PathBuf,
    identity: String,
    max_entries: usize,
    max_bytes: usize,
    journal: Journal,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Shared process-bound private-state custody. Interrupted stages fail closed.
pub(crate) fn lock_directory(directory: &std::path::Path) -> io::Result<(File, bool)> {
    platform::fs::create_private_dir(directory)?;
    let path = directory.join("lock");
    let created = match platform::fs::create_private_file(&path) {
        Ok(file) => {
            drop(file);
            true
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => false,
        Err(error) => return Err(error),
    };
    let lock = OpenOptions::new().read(true).write(true).open(&path)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        // Windows byte locks can deny reads through another handle, even
        // when the locked file is empty. Validate inside the bounded retry;
        // a custody failure still returns explicitly if it persists.
        if let Err(error) = platform::fs::read_private_file(&path, 0) {
            if std::time::Instant::now() >= deadline {
                return Err(error);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            continue;
        }
        match lock.try_lock() {
            Ok(()) => {
                for entry in std::fs::read_dir(directory)? {
                    if entry?
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".ghd-write-")
                    {
                        return Err(invalid("interrupted cache publication requires recovery"));
                    }
                }
                return Ok((lock, created));
            }
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "cache snapshot is busy",
                ));
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        }
    }
}

impl Store {
    pub fn new(
        directory: PathBuf,
        identity: String,
        max_entries: usize,
        max_bytes: usize,
        journal: Journal,
    ) -> Self {
        Self {
            directory,
            identity,
            max_entries,
            max_bytes,
            journal,
        }
    }

    fn lock(&self) -> io::Result<File> {
        lock_directory(&self.directory).map(|(lock, _)| lock)
    }

    fn load(&self, epoch: u64) -> io::Result<Cache<Vec<u8>>> {
        let mut cache = Cache::new(self.max_entries, self.max_bytes.saturating_sub(256));
        let bytes =
            match platform::fs::read_private_file(&self.directory.join("snapshot"), self.max_bytes)
            {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(cache),
                Err(error) => return Err(error),
            };
        let envelope =
            Envelope::decode(bytes.as_slice()).map_err(|_| invalid("corrupt cache envelope"))?;
        if kernal_api::hash::blake3_bytes(&envelope.body)
            .as_bytes()
            .as_slice()
            != envelope.checksum
        {
            return Err(invalid("cache snapshot checksum mismatch"));
        }
        let snapshot = Snapshot::decode(envelope.body.as_slice())
            .map_err(|_| invalid("corrupt cache snapshot"))?;
        if snapshot.version != 1 || snapshot.identity != self.identity {
            return Err(invalid("incompatible cache snapshot identity or schema"));
        }
        if snapshot.records.len() > self.max_entries {
            return Err(invalid("cache snapshot exceeds entry capacity"));
        }
        if snapshot.epoch != epoch {
            return Ok(cache);
        }
        for record in snapshot.records {
            if record.key.len() > 80 || cache.get(&record.key).is_some() {
                return Err(invalid("invalid or duplicate cache record key"));
            }
            let weight = record.encoded_len().saturating_add(8);
            let previous = cache.usage().0;
            if !cache.insert(record.key, record.body, weight) || cache.usage().0 != previous + 1 {
                return Err(invalid("cache snapshot exceeds byte capacity"));
            }
        }
        Ok(cache)
    }

    pub fn get(&self, key: &str, epoch: u64) -> io::Result<Option<Vec<u8>>> {
        self.journal.confirm(epoch)?;
        if self.max_entries == 0 || self.max_bytes <= 256 {
            return Ok(None);
        }
        let _lock = self.lock()?;
        let mut cache = self.load(epoch)?;
        self.journal.confirm(epoch)?;
        Ok(cache.get(key).cloned())
    }

    /// The original read epoch travels with its bytes, never the latest epoch.
    pub fn put(&self, key: &str, body: Vec<u8>, epoch: u64) -> io::Result<()> {
        self.update(key, Some(body), epoch)
    }

    pub fn remove(&self, key: &str, epoch: u64) -> io::Result<()> {
        self.update(key, None, epoch)
    }

    fn update(&self, key: &str, body: Option<Vec<u8>>, epoch: u64) -> io::Result<()> {
        self.journal.confirm(epoch)?;
        if self.max_entries == 0 || self.max_bytes <= 256 {
            return Ok(());
        }
        let _lock = self.lock()?;
        let mut cache = self.load(epoch)?;
        if let Some(body) = body {
            let record = Record {
                key: key.into(),
                body,
            };
            let weight = record.encoded_len().saturating_add(8);
            cache.insert(record.key, record.body, weight);
        } else {
            cache.remove(key);
        }
        self.publish(&cache, epoch)
    }

    fn publish(&self, cache: &Cache<Vec<u8>>, epoch: u64) -> io::Result<()> {
        let snapshot = Snapshot {
            version: 1,
            identity: self.identity.clone(),
            epoch,
            records: cache
                .ordered()
                .map(|(key, body)| Record {
                    key: key.into(),
                    body: body.clone(),
                })
                .collect(),
        };
        let body = snapshot.encode_to_vec();
        let checksum = kernal_api::hash::blake3_bytes(&body).as_bytes().to_vec();
        let bytes = Envelope { body, checksum }.encode_to_vec();
        if bytes.len() > self.max_bytes {
            return Err(invalid("encoded cache exceeds byte capacity"));
        }
        self.journal.confirm(epoch)?;
        platform::fs::write_private(&self.directory.join("snapshot"), &bytes)?;
        self.journal.confirm(epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        store: Store,
    }
    impl Fixture {
        fn new(entries: usize, bytes: usize) -> Self {
            let root = std::env::temp_dir().join(format!(
                "ghd-durable-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            assert!(!root.exists(), "new owned fixture");
            let journal = Journal::at(root.join("journal"));
            let store = Store::new(
                root.join("cache"),
                "fixture-identity".into(),
                entries,
                bytes,
                journal,
            );
            Self { root, store }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.root.exists() {
                std::fs::remove_dir_all(&self.root).expect("fixture cleanup");
            }
        }
    }

    #[test]
    fn restart_preserves_bounded_bytes_but_offline_mutations_fence_old_snapshots() {
        let fixture = Fixture::new(2, 1024);
        let store = &fixture.store;
        store
            .put("rest:a", b"old body".to_vec(), 0)
            .expect("persist");
        assert_eq!(
            store.get("rest:a", 0).expect("restart read"),
            Some(b"old body".to_vec())
        );
        // ghx #27: no daemon is needed to record this mutation.
        let first = store.journal.begin().expect("offline write");
        let second = store.journal.begin().expect("overlapping write");
        first.finish().expect("first complete");
        assert!(store.get("rest:a", 0).is_err(), "overlap remains fenced");
        second.finish().expect("second complete");
        let epoch = store.journal.epoch().expect("settled");
        assert_eq!(
            store.get("rest:a", epoch).expect("restart after write"),
            None
        );
        assert!(store.put("rest:a", b"late old result".to_vec(), 0).is_err());
        store
            .put("rest:b", b"new body".to_vec(), epoch)
            .expect("new bytes");
        assert_eq!(store.get("rest:a", epoch).expect("no relabel"), None);
    }

    #[test]
    fn corrupt_schema_identity_and_failed_publication_are_explicit() {
        let fixture = Fixture::new(2, 1024);
        let store = &fixture.store;
        store.put("rest:a", vec![1], 0).expect("persist");
        let other = Store::new(
            store.directory.clone(),
            "other-identity".into(),
            2,
            1024,
            store.journal.clone(),
        );
        assert!(other.get("rest:a", 0).is_err());
        let path = store.directory.join("snapshot");
        let bytes = platform::fs::read_private_file(&path, 1024).expect("private");
        let mut envelope = Envelope::decode(bytes.as_slice()).expect("envelope");
        let mut snapshot = Snapshot::decode(envelope.body.as_slice()).expect("snapshot");
        snapshot.version = 999;
        envelope.body = snapshot.encode_to_vec();
        envelope.checksum = kernal_api::hash::blake3_bytes(&envelope.body)
            .as_bytes()
            .to_vec();
        platform::fs::write_private(&path, &envelope.encode_to_vec()).expect("future schema");
        assert!(store.get("rest:a", 0).is_err());
        platform::fs::write_private(&path, b"corrupt").expect("corruption");
        assert!(store.get("rest:a", 0).is_err());
        assert!(
            store.put("rest:b", vec![2], 0).is_err(),
            "never overwrite corrupt custody silently"
        );
    }

    #[test]
    fn disk_limits_and_abandoned_intents_remain_fail_closed() {
        let fixture = Fixture::new(1, 1024);
        let store = &fixture.store;
        for number in 0..20 {
            store
                .put(&format!("rest:{number}"), vec![42; 100], 0)
                .expect("bounded publication");
        }
        assert_eq!(store.get("rest:0", 0).expect("evicted"), None);
        assert!(
            std::fs::metadata(store.directory.join("snapshot"))
                .expect("snapshot")
                .len()
                <= 1024
        );
        store
            .put("rest:19", vec![42; 2048], 0)
            .expect("oversized is not stored");
        assert_eq!(store.get("rest:19", 0).expect("oversized"), None);
        drop(store.journal.begin().expect("crash after intent"));
        assert!(store.get("rest:19", 0).is_err());
    }

    #[test]
    fn failed_publication_never_relabels_a_prewrite_snapshot() {
        let fixture = Fixture::new(2, 4096);
        let store = &fixture.store;
        store
            .put("rest:a", b"old bytes".to_vec(), 0)
            .expect("old snapshot");
        let path = store.directory.join("snapshot");
        let backup = store.directory.join("owned-backup");
        std::fs::rename(&path, &backup).expect("controlled filesystem failure");
        std::fs::create_dir(&path).expect("block atomic replacement");
        let mut cache = Cache::new(2, 3840);
        cache.insert("rest:b".into(), b"new bytes".to_vec(), 128);
        assert!(
            store.publish(&cache, 0).is_err(),
            "publication failure propagates"
        );
        std::fs::remove_dir(&path).expect("owned blocker");
        std::fs::rename(&backup, &path).expect("old snapshot survives");
        assert_eq!(
            store.get("rest:a", 0).expect("old generation"),
            Some(b"old bytes".to_vec())
        );
        store
            .journal
            .begin()
            .expect("offline mutation")
            .finish()
            .expect("completed");
        let epoch = store.journal.epoch().expect("new generation");
        assert_eq!(store.get("rest:a", epoch).expect("restart"), None);
    }

    #[test]
    fn concurrent_publications_do_not_lose_other_readers_records() {
        let fixture = Fixture::new(8, 4096);
        std::thread::scope(|scope| {
            for group in 0..2 {
                let store = &fixture.store;
                scope.spawn(move || {
                    for number in 0..4 {
                        let deadline =
                            std::time::Instant::now() + std::time::Duration::from_secs(10);
                        loop {
                            match store.put(&format!("rest:{group}{number}"), vec![42; 100], 0) {
                                Ok(()) => break,
                                Err(error)
                                    if error.kind() == io::ErrorKind::WouldBlock
                                        && std::time::Instant::now() < deadline =>
                                {
                                    std::thread::sleep(std::time::Duration::from_millis(5));
                                }
                                Err(error) => panic!("serialized publication: {error}"),
                            }
                        }
                    }
                });
            }
        });
        for group in 0..2 {
            for number in 0..4 {
                assert_eq!(
                    fixture
                        .store
                        .get(&format!("rest:{group}{number}"), 0)
                        .expect("retained"),
                    Some(vec![42; 100])
                );
            }
        }
    }

    #[test]
    fn contended_snapshot_lock_fails_without_unbounded_waiting() {
        let fixture = Fixture::new(2, 4096);
        fixture.store.put("rest:a", vec![42], 0).expect("snapshot");
        std::thread::scope(|scope| {
            let held = fixture
                .store
                .lock()
                .expect("another publisher owns custody");
            let (sender, receiver) = std::sync::mpsc::channel();
            let store = &fixture.store;
            let pending = scope.spawn(move || sender.send(store.get("rest:a", 0)).expect("report"));
            let result = receiver.recv_timeout(std::time::Duration::from_secs(5));
            drop(held); // Also releases a regressed blocking implementation before join.
            pending.join().expect("reader joined");
            assert!(result.expect("bounded lock acquisition").is_err());
        });
    }

    #[test]
    fn interrupted_stages_are_rejected_before_more_disk_data_can_accumulate() {
        let fixture = Fixture::new(2, 4096);
        fixture.store.put("rest:a", vec![42], 0).expect("snapshot");
        let stage = fixture.store.directory.join(".ghd-write-interrupted");
        std::fs::create_dir(&stage).expect("interrupted publication fixture");
        let error = fixture
            .store
            .get("rest:a", 0)
            .expect_err("recovery required");
        assert!(error.to_string().contains("requires recovery"));
        assert!(fixture.store.put("rest:b", vec![43], 0).is_err());
        assert!(
            stage.exists(),
            "never silently discard unproven staged bytes"
        );
        assert_eq!(
            std::fs::read_dir(&fixture.store.directory)
                .expect("directory")
                .count(),
            3
        );
    }
}
