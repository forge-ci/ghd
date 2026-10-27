//! A shared write journal fences every daemon's cached responses.
//!
//! A write intent is recorded before dispatch and completed only after the
//! writer has stopped. Dropping an intent deliberately leaves it unresolved:
//! a crash or an uncertain upstream result must not restore cache authority.

use crate::platform;
use prost::Message;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;

const VERSION: u32 = 1;
const MAX_RECORD: u64 = 16 * 1024;
const MAX_WRITERS: usize = 128;

#[derive(Clone)]
pub struct Journal {
    directory: PathBuf,
}

#[derive(Clone, PartialEq, Message)]
struct Record {
    #[prost(uint32, tag = "1")]
    version: u32,
    #[prost(uint64, tag = "2")]
    epoch: u64,
    #[prost(uint64, repeated, tag = "3")]
    active: Vec<u64>,
}

pub struct WriteIntent {
    journal: Journal,
    id: u64,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

impl Journal {
    pub fn global() -> io::Result<Self> {
        Ok(Self::at(platform::fs::state_dir()?.join("coherence-v1")))
    }

    pub fn at(directory: PathBuf) -> Self {
        Self { directory }
    }

    fn record(&self) -> io::Result<Record> {
        let bytes =
            platform::fs::read_private_file(&self.directory.join("journal"), MAX_RECORD as usize)?;
        let record =
            Record::decode(bytes.as_slice()).map_err(|_| invalid("corrupt write journal"))?;
        if record.version != VERSION || record.active.len() > MAX_WRITERS {
            return Err(invalid("unsupported write journal"));
        }
        Ok(record)
    }

    fn save(&self, record: &Record) -> io::Result<()> {
        platform::fs::write_private(&self.directory.join("journal"), &record.encode_to_vec())
    }

    fn lock_writer(&self) -> io::Result<(File, Record)> {
        platform::fs::create_private_dir(&self.directory)?;
        let path = self.directory.join("lock");
        let (lock, created) = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(lock) => (lock, true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (
                OpenOptions::new().read(true).write(true).open(&path)?,
                false,
            ),
            Err(error) => return Err(error),
        };
        lock.lock()?;
        let record = if created {
            if self.directory.join("journal").try_exists()? {
                return Err(invalid("write journal lost its custody lock"));
            }
            Record {
                version: VERSION,
                epoch: 0,
                active: Vec::new(),
            }
        } else {
            self.record()?
        };
        Ok((lock, record))
    }

    /// No writes have ever happened when both journal and lock are absent.
    /// Once a lock exists, a missing/corrupt record fails closed.
    pub fn epoch(&self) -> io::Result<u64> {
        let lock = match File::open(self.directory.join("lock")) {
            Ok(lock) => lock,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if self.directory.join("journal").try_exists()? {
                    return Err(invalid("write journal lock is missing"));
                }
                return Ok(0);
            }
            Err(error) => return Err(error),
        };
        lock.lock_shared()?;
        let record = self.record()?;
        if !record.active.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "GitHub write is active or uncertain; cached reads are fenced until its intent is resolved",
            ));
        }
        Ok(record.epoch)
    }

    pub fn confirm(&self, epoch: u64) -> io::Result<()> {
        if self.epoch()? != epoch {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "GitHub changed while reading; retry the read",
            ));
        }
        Ok(())
    }

    pub fn begin(&self) -> io::Result<WriteIntent> {
        let (_lock, mut record) = self.lock_writer()?;
        if record.active.len() >= MAX_WRITERS {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "too many outstanding GitHub writes",
            ));
        }
        record.epoch = record
            .epoch
            .checked_add(1)
            .ok_or_else(|| invalid("write journal exhausted"))?;
        let id = record.epoch;
        record.active.push(id);
        self.save(&record)?;
        Ok(WriteIntent {
            journal: self.clone(),
            id,
        })
    }
}

impl WriteIntent {
    /// Call only after the subprocess exited or an upstream response proved
    /// completion. On uncertainty leave the intent present and fail explicitly.
    pub fn finish(self) -> io::Result<()> {
        let (_lock, mut record) = self.journal.lock_writer()?;
        let index = record
            .active
            .iter()
            .position(|id| *id == self.id)
            .ok_or_else(|| invalid("GitHub write intent is missing"))?;
        record.active.remove(index);
        record.epoch = record
            .epoch
            .checked_add(1)
            .ok_or_else(|| invalid("write journal exhausted"))?;
        self.journal.save(&record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(Journal);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "ghd-journal-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            assert!(!path.exists(), "fixture must own a new directory");
            Self(Journal::at(path))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.0.directory.exists() {
                std::fs::remove_dir_all(&self.0.directory).expect("clean journal fixture");
            }
        }
    }

    #[test]
    fn writes_fence_old_reads_across_overlap_and_restart() {
        let fixture = Fixture::new();
        let journal = &fixture.0;
        assert_eq!(journal.epoch().expect("initial"), 0);
        let first = journal.begin().expect("first intent");
        assert!(journal.epoch().is_err());
        let second = journal.begin().expect("overlapping intent");
        first.finish().expect("first complete");
        let restarted = Journal::at(journal.directory.clone());
        assert!(
            restarted.epoch().is_err(),
            "second write still fences reads after restart"
        );
        second.finish().expect("second complete");
        let current = restarted.epoch().expect("idle");
        assert!(current > 0);
        assert!(
            journal.confirm(0).is_err(),
            "pre-write data cannot become authoritative"
        );
        journal.confirm(current).expect("unchanged");
    }

    #[test]
    fn abandoned_intents_and_corruption_never_restore_cache_authority() {
        let fixture = Fixture::new();
        let journal = &fixture.0;
        let intent = journal.begin().expect("intent recorded before dispatch");
        drop(intent);
        assert!(Journal::at(journal.directory.clone()).epoch().is_err());
        platform::fs::write_private(&journal.directory.join("journal"), b"corrupt")
            .expect("corrupt fixture");
        assert!(journal.epoch().is_err());
        assert!(
            journal.begin().is_err(),
            "do not overwrite corrupt write custody"
        );
    }
}
