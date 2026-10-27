//! Private cross-process governor reservations and durable cooldown.

use crate::{durable::lock_directory, governor::State, platform};
use prost::Message;
use std::{io, path::PathBuf};

const MAX_RECORD: usize = 16 * 1024;
const LEASE_SECONDS: u64 = 180;

#[derive(Clone, PartialEq, Message)]
struct Charge {
    #[prost(string, tag = "1")]
    name: String,
    #[prost(uint64, tag = "2")]
    reset: u64,
}

#[derive(Clone, PartialEq, Message)]
struct Pending {
    #[prost(uint64, tag = "1")]
    id: u64,
    #[prost(uint64, tag = "2")]
    deadline: u64,
    #[prost(bool, tag = "3")]
    essential: bool,
    #[prost(message, repeated, tag = "4")]
    charged: Vec<Charge>,
    #[prost(string, tag = "5")]
    category: String,
    #[prost(bool, tag = "6")]
    measurement: bool,
}

#[derive(Clone, PartialEq, Message)]
struct Record {
    #[prost(uint32, tag = "1")]
    version: u32,
    #[prost(string, tag = "2")]
    identity: String,
    #[prost(message, required, tag = "3")]
    state: State,
    #[prost(message, repeated, tag = "4")]
    pending: Vec<Pending>,
    #[prost(uint64, tag = "5")]
    next: u64,
}

#[derive(Clone, PartialEq, Message)]
struct Envelope {
    #[prost(bytes = "vec", tag = "1")]
    body: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    checksum: Vec<u8>,
}

#[derive(Clone)]
pub struct Ledger {
    directory: PathBuf,
    identity: String,
    reserve: u64,
}

pub struct Reservation {
    id: u64,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn busy(message: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::WouldBlock, message.to_string())
}

fn checksum(body: &[u8]) -> Vec<u8> {
    let mut material = b"ghd/governor-state/v1\0".to_vec();
    material.extend_from_slice(body);
    kernal_api::hash::blake3_bytes(&material)
        .as_bytes()
        .to_vec()
}

fn refund(state: &mut State, charges: &[Charge]) {
    for charge in charges {
        if let Some(budget) = state.buckets.get_mut(&charge.name)
            && budget.reset == charge.reset
        {
            budget.remaining = budget.remaining.saturating_add(1);
        }
    }
}

impl Ledger {
    pub fn at(directory: PathBuf, identity: String, reserve: u64) -> Self {
        Self {
            directory,
            identity,
            reserve,
        }
    }

    fn load(&self) -> io::Result<Record> {
        let bytes = platform::fs::read_private_file(&self.directory.join("state"), MAX_RECORD)?;
        let envelope =
            Envelope::decode(bytes.as_slice()).map_err(|_| invalid("corrupt governor envelope"))?;
        if checksum(&envelope.body) != envelope.checksum {
            return Err(invalid("governor checksum mismatch"));
        }
        let record = Record::decode(envelope.body.as_slice())
            .map_err(|_| invalid("corrupt governor state"))?;
        if record.version != 1 || record.identity != self.identity || record.pending.len() > 4 {
            return Err(invalid(
                "incompatible governor schema, identity or capacity",
            ));
        }
        record.state.validate()?;
        let ids: std::collections::BTreeSet<_> =
            record.pending.iter().map(|pending| pending.id).collect();
        if ids.len() != record.pending.len()
            || record.pending.iter().any(|pending| {
                pending.id == 0
                    || pending.id > record.next
                    || pending.charged.len() > 16
                    || pending.category.is_empty()
                    || pending.category.len() > 32
                    || !pending
                        .category
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    || pending
                        .charged
                        .iter()
                        .any(|charge| !record.state.buckets.contains_key(&charge.name))
            })
        {
            return Err(invalid("invalid governor reservation state"));
        }
        Ok(record)
    }

    fn record(&self, created: bool) -> io::Result<Record> {
        match self.load() {
            Ok(_) if created => Err(invalid("governor lost its custody lock")),
            Ok(record) => Ok(record),
            Err(error) if created && error.kind() == io::ErrorKind::NotFound => Ok(Record {
                version: 1,
                identity: self.identity.clone(),
                state: State::default(),
                pending: Vec::new(),
                next: 0,
            }),
            Err(error) => Err(error),
        }
    }

    fn save(&self, record: &Record) -> io::Result<()> {
        let body = record.encode_to_vec();
        let checksum = checksum(&body);
        let bytes = Envelope { body, checksum }.encode_to_vec();
        if bytes.len() > MAX_RECORD {
            return Err(invalid("governor state exceeds capacity"));
        }
        platform::fs::write_private(&self.directory.join("state"), &bytes)
    }

    pub fn reserve(
        &self,
        category: &str,
        essential: bool,
        measurement: bool,
        now: u64,
    ) -> io::Result<Reservation> {
        if category.is_empty()
            || category.len() > 32
            || !category
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid governor category",
            ));
        }
        let (_lock, created) = lock_directory(&self.directory)?;
        let mut record = self.record(created)?;
        let recovery = record
            .pending
            .iter()
            .filter(|pending| pending.deadline <= now)
            .map(|pending| pending.deadline.saturating_add(60))
            .max();
        record.pending.retain(|pending| pending.deadline > now);
        if let Some(until) = recovery {
            record.state.blocked_until = record.state.blocked_until.max(until);
            self.save(&record)?;
        }
        if !measurement {
            record
                .state
                .admit(category, essential, self.reserve, now)
                .map_err(|error| io::Error::new(io::ErrorKind::WouldBlock, error))?;
        }
        if record.pending.len() >= 4
            || !essential
                && record
                    .pending
                    .iter()
                    .filter(|pending| !pending.essential)
                    .count()
                    >= 3
        {
            return Err(busy("shared upstream capacity is busy; retry"));
        }
        let mut charged = Vec::new();
        if !measurement {
            for (name, budget) in &mut record.state.buckets {
                if (name == category || category == "unknown") && budget.reset > now {
                    if budget.remaining > 0 {
                        charged.push(Charge {
                            name: name.clone(),
                            reset: budget.reset,
                        });
                    }
                    budget.remaining = budget.remaining.saturating_sub(1);
                }
            }
        }
        record.next = record
            .next
            .checked_add(1)
            .ok_or_else(|| invalid("governor operation IDs exhausted"))?;
        record.pending.push(Pending {
            id: record.next,
            deadline: now.saturating_add(LEASE_SECONDS),
            essential,
            charged,
            category: category.into(),
            measurement,
        });
        self.save(&record)?;
        Ok(Reservation { id: record.next })
    }

    pub fn finish(
        &self,
        reservation: Reservation,
        status: u16,
        headers: &str,
        body: &[u8],
        now: u64,
    ) -> io::Result<bool> {
        let (_lock, created) = lock_directory(&self.directory)?;
        let mut record = self.record(created)?;
        let index = record
            .pending
            .iter()
            .position(|pending| pending.id == reservation.id)
            .ok_or_else(|| invalid("governor operation lease is missing"))?;
        let pending = record.pending.remove(index);
        if status == 304 {
            refund(&mut record.state, &pending.charged);
        }
        let previous = record.state.buckets.clone();
        let observed_body = if pending.category == "graphql" {
            body
        } else {
            &[]
        };
        let headers = if crate::governor::header(headers, "x-ratelimit-resource").is_none() {
            format!("{headers}\nx-ratelimit-resource: {}", pending.category)
        } else {
            headers.to_owned()
        };
        let failures = record.state.failures;
        let limited = record.state.observe(status, &headers, observed_body, now);
        if pending.measurement && status == 200 {
            record.state.observe_resources(body);
            if !limited {
                record.state.failures = failures;
            }
        }
        // Same-window measurements can arrive out of order. Keep a conservative
        // estimate with minimum reservations; this is not measured point usage.
        for (name, budget) in &mut record.state.buckets {
            if let Some(old) = previous.get(name)
                && old.reset == budget.reset
            {
                budget.remaining = budget.remaining.min(old.remaining);
            }
        }
        self.save(&record)?;
        if now >= pending.deadline {
            return Err(invalid("governor operation lease expired"));
        }
        Ok(limited)
    }

    pub fn cancel_before_dispatch(&self, reservation: Reservation, _now: u64) -> io::Result<()> {
        let (_lock, created) = lock_directory(&self.directory)?;
        let mut record = self.record(created)?;
        let index = record
            .pending
            .iter()
            .position(|pending| pending.id == reservation.id)
            .ok_or_else(|| invalid("governor operation lease is missing"))?;
        let pending = record.pending.remove(index);
        refund(&mut record.state, &pending.charged);
        self.save(&record)
    }

    pub fn snapshot(&self) -> io::Result<State> {
        match self.load() {
            Ok(record) => Ok(record.state),
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    && !self.directory.join("lock").try_exists()? =>
            {
                Ok(State::default())
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture {
        root: PathBuf,
        ledger: Ledger,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "ghd-governor-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            assert!(!root.exists(), "owned fixture");
            Self {
                ledger: Ledger::at(root.clone(), "fixture-identity".into(), 100),
                root,
            }
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
    fn cooldown_survives_restart_and_essential_calls_do_not_bypass_it() {
        let fixture = Fixture::new();
        let reservation = fixture
            .ledger
            .reserve("core", false, false, 100)
            .expect("first dispatch");
        fixture
            .ledger
            .finish(reservation, 429, "Retry-After: 120", b"{}", 100)
            .expect("throttle saved");
        let restarted = Ledger::at(fixture.root.clone(), "fixture-identity".into(), 100);
        assert!(restarted.reserve("graphql", true, false, 219).is_err());
        let measurement = restarted
            .reserve("core", true, true, 219)
            .expect("free introspection");
        restarted
            .finish(measurement, 200, "", b"{}", 219)
            .expect("measurement cannot cancel cooldown");
        assert!(restarted.reserve("graphql", true, false, 219).is_err());
        let allowed = restarted
            .reserve("graphql", true, false, 220)
            .expect("deadline reached");
        restarted
            .cancel_before_dispatch(allowed, 220)
            .expect("not dispatched");
    }
    #[test]
    fn reserve_floors_leave_capacity_for_essential_work_and_expire_at_reset() {
        let fixture = Fixture::new();
        let first = fixture
            .ledger
            .reserve("core", false, false, 100)
            .expect("unknown budget");
        fixture
            .ledger
            .finish(
                first,
                200,
                "X-RateLimit-Resource: core\nX-RateLimit-Remaining: 99\nX-RateLimit-Reset: 200",
                b"{}",
                100,
            )
            .expect("observed floor");
        assert!(fixture.ledger.reserve("core", false, false, 101).is_err());
        let essential = fixture
            .ledger
            .reserve("core", true, false, 101)
            .expect("protected reserve");
        fixture
            .ledger
            .cancel_before_dispatch(essential, 101)
            .expect("complete");
        let reset = fixture
            .ledger
            .reserve("core", false, false, 200)
            .expect("reset");
        fixture
            .ledger
            .cancel_before_dispatch(reset, 200)
            .expect("complete");
    }
    #[test]
    fn global_slots_survive_cancellation_and_preserve_one_essential_slot() {
        let fixture = Fixture::new();
        let mut pending = Vec::new();
        for _ in 0..3 {
            pending.push(
                fixture
                    .ledger
                    .reserve("core", false, false, 100)
                    .expect("routine slot"),
            );
        }
        assert!(fixture.ledger.reserve("core", false, false, 100).is_err());
        pending.push(
            fixture
                .ledger
                .reserve("graphql", true, false, 100)
                .expect("essential slot"),
        );
        assert!(fixture.ledger.reserve("graphql", true, false, 100).is_err());
        drop(pending); // Process exit/cancellation must not immediately free live work.
        assert!(fixture.ledger.reserve("core", true, false, 279).is_err());
        assert!(
            fixture.ledger.reserve("core", true, false, 280).is_err(),
            "expired unknown work creates cooldown"
        );
        let fresh = fixture
            .ledger
            .reserve("core", true, false, 340)
            .expect("bounded recovery window elapsed");
        fixture
            .ledger
            .cancel_before_dispatch(fresh, 340)
            .expect("complete");
    }
    #[test]
    fn corrupt_missing_or_wrong_identity_state_never_resets_policy_silently() {
        let fixture = Fixture::new();
        let initial = fixture
            .ledger
            .reserve("core", false, false, 100)
            .expect("initial");
        fixture
            .ledger
            .cancel_before_dispatch(initial, 100)
            .expect("complete");
        let other = Ledger::at(fixture.root.clone(), "other".into(), 0);
        assert!(other.reserve("core", true, false, 100).is_err());
        let path = fixture.root.join("state");
        std::fs::rename(&path, fixture.root.join("owned-backup")).expect("missing state");
        assert!(fixture.ledger.reserve("core", true, false, 100).is_err());
        platform::fs::write_private(&path, b"corrupt").expect("corrupt fixture");
        assert!(fixture.ledger.reserve("core", true, false, 100).is_err());
    }

    #[test]
    fn a_free_304_refunds_its_own_reservation_without_draining_the_floor() {
        let fixture = Fixture::new();
        let first = fixture
            .ledger
            .reserve("core", false, false, 100)
            .expect("first");
        let headers =
            "X-RateLimit-Resource: core\nX-RateLimit-Remaining: 101\nX-RateLimit-Reset: 200";
        fixture
            .ledger
            .finish(first, 200, headers, b"{}", 100)
            .expect("observed");
        for _ in 0..3 {
            let conditional = fixture
                .ledger
                .reserve("core", false, false, 100)
                .expect("room above floor");
            fixture
                .ledger
                .finish(conditional, 304, headers, &[], 100)
                .expect("free conditional");
        }
        assert_eq!(
            fixture.ledger.snapshot().expect("state").buckets["core"].remaining,
            101
        );
    }

    #[test]
    fn introspection_updates_other_categories_and_rest_content_cannot_spoof_graphql_errors() {
        let fixture = Fixture::new();
        let first = fixture
            .ledger
            .reserve("core", false, false, 100)
            .expect("first");
        assert!(
            !fixture
                .ledger
                .finish(
                    first,
                    200,
                    "",
                    br#"{"errors":[{"type":"RATE_LIMITED"}]}"#,
                    100
                )
                .expect("REST content")
        );
        let measurement = fixture
            .ledger
            .reserve("core", true, true, 100)
            .expect("measurement");
        fixture
            .ledger
            .finish(
                measurement,
                200,
                "",
                br#"{"resources":{"graphql":{"remaining":0,"reset":200}}}"#,
                100,
            )
            .expect("all provider resources");
        assert!(fixture.ledger.reserve("graphql", true, false, 100).is_err());
    }

    #[test]
    fn valid_cache_envelopes_cannot_reset_governor_policy() {
        #[derive(Clone, PartialEq, Message)]
        struct EmptyCacheSnapshot {
            #[prost(uint32, tag = "1")]
            version: u32,
            #[prost(string, tag = "2")]
            identity: String,
        }
        let fixture = Fixture::new();
        let first = fixture
            .ledger
            .reserve("core", false, false, 100)
            .expect("initial");
        fixture
            .ledger
            .finish(first, 429, "Retry-After: 120", b"{}", 100)
            .expect("cooldown");
        let body = EmptyCacheSnapshot {
            version: 1,
            identity: "fixture-identity".into(),
        }
        .encode_to_vec();
        let wrong = Envelope {
            checksum: kernal_api::hash::blake3_bytes(&body).as_bytes().to_vec(),
            body,
        }
        .encode_to_vec();
        platform::fs::write_private(&fixture.root.join("state"), &wrong)
            .expect("wrong record fixture");
        assert!(
            fixture.ledger.snapshot().is_err(),
            "cache records cannot restore default governor policy"
        );
        assert!(fixture.ledger.reserve("core", true, false, 101).is_err());
    }
}
