//! One governed dispatch pool and durable policy per credential identity.

use ghd_core::{
    governor::Refusal,
    governor_ledger::{Ledger, Reservation},
};
use kernal_api::{
    async_engine::{self, Semaphore, SemaphorePermit},
    json::Value,
};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

kernal_api::task_local! { pub(crate) static CALLER: String = CALLER_STORAGE; }

#[derive(Clone, Copy)]
pub(crate) enum Event {
    Request,
    Hit,
    Http,
    Process,
    Refused,
    Revalidated,
    Uncertain,
    Throttled,
}
#[derive(Clone, Default)]
struct Counts {
    values: [u64; 8],
}
impl Counts {
    fn add(&mut self, event: Event) {
        let value = &mut self.values[event as usize];
        *value = value.saturating_add(1);
    }
    fn json(&self) -> Value {
        Value::Object(
            [
                "requests",
                "cache_hits",
                "http_dispatches",
                "gh_dispatches",
                "refused",
                "revalidated",
                "uncertain",
                "throttled",
            ]
            .into_iter()
            .zip(self.values)
            .map(|(name, value)| (name.into(), Value::Unsigned(value)))
            .collect(),
        )
    }
}

pub(crate) fn caller() -> String {
    CALLER
        .try_with(Clone::clone)
        .unwrap_or_else(|_| "anonymous".into())
}
pub(crate) fn now() -> io::Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .map_err(io::Error::other)
}

pub(crate) struct Control {
    ledger: Ledger,
    total: Semaphore,
    routine: Semaphore,
    metrics: Mutex<(Counts, BTreeMap<String, Counts>)>,
}

impl Control {
    pub(crate) fn new(identity: &ghd_core::identity::Identity) -> io::Result<Arc<Self>> {
        let reserve = u64::try_from(crate::retention::limit("GHD_RATE_RESERVE", 0)?)
            .map_err(io::Error::other)?;
        Ok(Self::at(
            ghd_core::endpoint::directory(identity)?.join("governor-v1"),
            identity.key().into(),
            reserve,
        ))
    }
    pub(crate) fn at(directory: std::path::PathBuf, identity: String, reserve: u64) -> Arc<Self> {
        Arc::new(Self {
            ledger: Ledger::at(directory, identity, reserve),
            total: Semaphore::new(4),
            routine: Semaphore::new(3),
            metrics: Mutex::new((Counts::default(), BTreeMap::new())),
        })
    }
    #[cfg(test)]
    pub(crate) fn available_for_test(&self) -> (usize, usize) {
        (
            self.total.available_permits(),
            self.routine.available_permits(),
        )
    }

    pub(crate) fn event(&self, who: &str, event: Event) {
        if let Ok(mut metrics) = self.metrics.lock() {
            metrics.0.add(event);
            let key = if metrics.1.len() >= 31 && !metrics.1.contains_key(who) {
                "overflow"
            } else {
                who
            };
            metrics.1.entry(key.into()).or_default().add(event);
        }
    }
    pub(crate) fn answer(&self, source: &str) {
        match source {
            "fresh" | "shared" => self.event(&caller(), Event::Hit),
            "revalidated" => self.event(&caller(), Event::Revalidated),
            _ => {}
        }
    }

    pub(crate) async fn enter(
        self: &Arc<Self>,
        category: &str,
        essential: bool,
        measurement: bool,
        event: Event,
    ) -> io::Result<Dispatch> {
        let result = async {
            let routine = if essential {
                None
            } else {
                Some(
                    async_engine::timeout(Duration::from_secs(1), self.routine.acquire())
                        .await
                        .map_err(|_| busy())?,
                )
            };
            let total = async_engine::timeout(Duration::from_secs(1), self.total.acquire())
                .await
                .map_err(|_| busy())?;
            let deadline = Instant::now() + Duration::from_secs(1);
            loop {
                let ledger = self.ledger.clone();
                let category = category.to_owned();
                let reservation = async_engine::launch_blocking(move || {
                    ledger.reserve(&category, essential, measurement, now()?)
                })
                .await
                .map_err(io::Error::other)?;
                match reservation {
                    Ok(reservation) => {
                        return Ok(Dispatch {
                            control: Arc::clone(self),
                            reservation: Some(reservation),
                            who: caller(),
                            event,
                            started: false,
                            completed: false,
                            _routine: routine,
                            _total: total,
                        });
                    }
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock
                            && !error.get_ref().is_some_and(|source| source.is::<Refusal>())
                            && Instant::now() < deadline =>
                    {
                        async_engine::sleep(Duration::from_millis(5)).await;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        .await;
        if result.is_err() {
            self.event(&caller(), Event::Refused);
        }
        result
    }

    pub(crate) fn stats(&self) -> io::Result<Value> {
        let metrics = self
            .metrics
            .lock()
            .map_err(|_| io::Error::other("governor metrics poisoned"))?;
        let state = self.ledger.snapshot()?;
        let mut out = BTreeMap::from([
            ("totals".into(), metrics.0.json()),
            (
                "callers".into(),
                Value::Object(
                    metrics
                        .1
                        .iter()
                        .map(|(who, counts)| (who.clone(), counts.json()))
                        .collect(),
                ),
            ),
            ("blocked_until".into(), Value::Unsigned(state.blocked_until)),
        ]);
        out.insert(
            "budgets".into(),
            Value::Object(
                state
                    .buckets
                    .into_iter()
                    .map(|(name, budget)| {
                        (
                            name,
                            Value::Object(BTreeMap::from([
                                (
                                    "remaining_estimate".into(),
                                    Value::Unsigned(budget.remaining),
                                ),
                                ("reset".into(), Value::Unsigned(budget.reset)),
                            ])),
                        )
                    })
                    .collect(),
            ),
        );
        Ok(Value::Object(out))
    }
}

fn busy() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "shared upstream capacity is busy; retry",
    )
}

pub(crate) struct Dispatch {
    control: Arc<Control>,
    reservation: Option<Reservation>,
    who: String,
    event: Event,
    started: bool,
    completed: bool,
    _routine: Option<SemaphorePermit>,
    _total: SemaphorePermit,
}
impl Dispatch {
    fn cancel(mut self) -> io::Result<()> {
        let reservation = self
            .reservation
            .take()
            .ok_or_else(|| io::Error::other("missing governor reservation"))?;
        self.control
            .ledger
            .cancel_before_dispatch(reservation, now()?)?;
        self.completed = true;
        Ok(())
    }

    pub(crate) fn start(&mut self) {
        self.started = true;
        self.control.event(&self.who, self.event);
    }
    pub(crate) fn complete(mut self, status: u16, headers: &str, body: &[u8]) -> io::Result<()> {
        let reservation = self
            .reservation
            .take()
            .ok_or_else(|| io::Error::other("missing governor reservation"))?;
        if self
            .control
            .ledger
            .finish(reservation, status, headers, body, now()?)?
        {
            self.control.event(&self.who, Event::Throttled);
        }
        self.completed = true;
        Ok(())
    }
}
impl Drop for Dispatch {
    fn drop(&mut self) {
        if self.started && !self.completed {
            self.control.event(&self.who, Event::Uncertain);
        }
    }
}

pub(crate) async fn complete(
    dispatch: Dispatch,
    status: u16,
    headers: String,
    body: Vec<u8>,
) -> io::Result<Vec<u8>> {
    async_engine::launch_blocking(move || {
        dispatch.complete(status, &headers, &body)?;
        Ok(body)
    })
    .await
    .map_err(io::Error::other)?
}

pub(crate) async fn cancel(dispatch: Dispatch) -> io::Result<()> {
    async_engine::launch_blocking(move || dispatch.cancel())
        .await
        .map_err(io::Error::other)?
}
