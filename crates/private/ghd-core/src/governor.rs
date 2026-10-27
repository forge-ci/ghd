//! Budget floors and provider cooldowns, evaluated against an explicit clock.

use kernal_api::json::{self, Value};
use prost::Message;
use std::{collections::BTreeMap, io, time::UNIX_EPOCH};

#[derive(Clone, PartialEq, Message)]
pub struct Budget {
    #[prost(uint64, tag = "1")]
    pub remaining: u64,
    #[prost(uint64, tag = "2")]
    pub reset: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct State {
    #[prost(btree_map = "string, message", tag = "1")]
    pub buckets: BTreeMap<String, Budget>,
    #[prost(uint64, tag = "2")]
    pub blocked_until: u64,
    #[prost(uint32, tag = "3")]
    pub failures: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    Cooldown(u64),
    Budget(u64),
}
impl std::error::Error for Refusal {}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cooldown(until) => {
                write!(f, "provider cooldown until epoch {until}; retry later")
            }
            Self::Budget(until) => write!(
                f,
                "provider budget reserve until epoch {until}; retry later"
            ),
        }
    }
}

pub fn header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find_map(|(key, value)| key.eq_ignore_ascii_case(name).then(|| value.trim()))
}

fn number(headers: &str, name: &str) -> Option<u64> {
    header(headers, name).and_then(|value| value.parse().ok())
}

pub fn rest_category(endpoint: &str) -> &'static str {
    let path = endpoint
        .trim_start_matches('/')
        .split('?')
        .next()
        .unwrap_or_default();
    if path == "search/code" {
        "code_search"
    } else if path.starts_with("search/") {
        "search"
    } else {
        "core"
    }
}

fn graphql_limited(body: &[u8]) -> bool {
    let Ok(Value::Object(top)) = json::parse(body) else {
        return false;
    };
    let Some(Value::Array(errors)) = top.get("errors") else {
        return false;
    };
    errors.iter().any(|error| {
        let Value::Object(error) = error else {
            return false;
        };
        if error.get("type") == Some(&Value::String("RATE_LIMITED".into())) {
            return true;
        }
        if let Some(Value::Object(extensions)) = error.get("extensions")
            && extensions.get("code") == Some(&Value::String("RATE_LIMITED".into()))
        {
            return true;
        }
        let Some(Value::String(message)) = error.get("message") else {
            return false;
        };
        let message = message.to_ascii_lowercase();
        message.contains("api rate limit exceeded") || message.contains("secondary rate limit")
    })
}

impl State {
    pub fn validate(&self) -> io::Result<()> {
        if self.buckets.len() > 16
            || self.failures > 32
            || self.buckets.keys().any(|key| {
                key.is_empty()
                    || key.len() > 32
                    || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid governor state",
            ));
        }
        Ok(())
    }

    pub fn admit(
        &self,
        category: &str,
        essential: bool,
        reserve: u64,
        now: u64,
    ) -> Result<(), Refusal> {
        if self.blocked_until > now {
            return Err(Refusal::Cooldown(self.blocked_until));
        }
        for (name, budget) in &self.buckets {
            if (name == category || category == "unknown")
                && budget.reset > now
                && (budget.remaining == 0 || !essential && budget.remaining <= reserve)
            {
                return Err(Refusal::Budget(budget.reset));
            }
        }
        Ok(())
    }

    pub fn observe_budget(&mut self, name: &str, remaining: u64, reset: u64) {
        if !name.is_empty()
            && name.len() <= 32
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            && (self.buckets.contains_key(name) || self.buckets.len() < 16)
            && self.buckets.get(name).is_none_or(|old| reset >= old.reset)
        {
            self.buckets
                .insert(name.into(), Budget { remaining, reset });
        }
    }

    pub fn observe_resources(&mut self, body: &[u8]) {
        let Ok(Value::Object(top)) = json::parse(body) else {
            return;
        };
        let Some(Value::Object(resources)) = top.get("resources") else {
            return;
        };
        for (name, value) in resources.iter().take(16) {
            let Value::Object(value) = value else {
                continue;
            };
            let number = |key| match value.get(key) {
                Some(Value::Unsigned(value)) => Some(*value),
                Some(Value::Signed(value)) => u64::try_from(*value).ok(),
                _ => None,
            };
            if let (Some(remaining), Some(reset)) = (number("remaining"), number("reset")) {
                self.observe_budget(name, remaining, reset);
            }
        }
    }

    /// Records provider measurements, never an inferred per-command point cost.
    pub fn observe(&mut self, status: u16, headers: &str, body: &[u8], now: u64) -> bool {
        let remaining = number(headers, "x-ratelimit-remaining");
        let reset = number(headers, "x-ratelimit-reset");
        let retry = header(headers, "retry-after");
        if let (Some(name), Some(remaining), Some(reset)) =
            (header(headers, "x-ratelimit-resource"), remaining, reset)
        {
            self.observe_budget(name, remaining, reset);
        }
        let limited = status == 429
            || status == 200 && graphql_limited(body)
            || status == 403 && (retry.is_some() || remaining == Some(0));
        if limited {
            self.failures = self.failures.saturating_add(1).min(32);
            let delay = 60_u64
                .saturating_mul(1_u64 << (self.failures - 1).min(6))
                .min(3600);
            let hinted = retry
                .and_then(|retry| {
                    retry
                        .parse::<u64>()
                        .ok()
                        .map(|seconds| now.saturating_add(seconds.max(1)))
                        .or_else(|| {
                            httpdate::parse_http_date(retry)
                                .ok()?
                                .duration_since(UNIX_EPOCH)
                                .ok()
                                .map(|value| value.as_secs())
                        })
                })
                .filter(|until| *until > now);
            let reset = if remaining == Some(0) {
                reset.filter(|reset| *reset > now)
            } else {
                None
            };
            let until = match (hinted, reset) {
                (Some(retry), Some(reset)) => retry.max(reset),
                (Some(until), None) | (None, Some(until)) => until,
                (None, None) => now.saturating_add(delay),
            };
            self.blocked_until = self.blocked_until.max(until);
        } else if (200..400).contains(&status) {
            self.failures = 0;
        }
        limited
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn floors_reset_and_essential_calls_preserve_the_reserve() {
        let mut state = State::default();
        state.observe(
            200,
            "X-RateLimit-Resource: core\nX-RateLimit-Remaining: 100\nX-RateLimit-Reset: 200",
            b"{}",
            100,
        );
        assert_eq!(
            state.admit("core", false, 100, 100),
            Err(Refusal::Budget(200))
        );
        assert!(state.admit("core", true, 100, 100).is_ok());
        assert!(state.admit("graphql", false, 100, 100).is_ok());
        assert!(state.admit("core", false, 100, 200).is_ok());
        state.buckets.get_mut("core").expect("bucket").remaining = 0;
        assert!(state.admit("core", true, 100, 100).is_err());
    }
    #[test]
    fn throttling_differs_from_authorization_denial_and_honors_retry_dates() {
        let mut state = State::default();
        assert!(!state.observe(403, "", b"permission denied", 100));
        assert!(state.admit("core", false, 100, 100).is_ok());
        assert!(state.observe(429, "Retry-After: 120", b"{}", 100));
        assert_eq!(state.blocked_until, 220);
        assert!(state.admit("graphql", true, 0, 219).is_err());
        assert!(state.admit("graphql", true, 0, 220).is_ok());
        let mut state = State::default();
        assert!(state.observe(
            403,
            "Retry-After: Thu, 01 Jan 1970 00:05:00 GMT",
            b"{}",
            100
        ));
        assert_eq!(state.blocked_until, 300);
    }
    #[test]
    fn http_200_graphql_errors_and_depleted_403s_create_durable_cooldown_state() {
        let mut state = State::default();
        assert!(state.observe(
            200,
            "",
            br#"{"data":null,"errors":[{"type":"RATE_LIMITED","message":"no budget"}]}"#,
            100
        ));
        assert_eq!(state.blocked_until, 160);
        let restored = State::decode(state.encode_to_vec().as_slice()).expect("restart");
        restored.validate().expect("bounded");
        assert!(restored.admit("core", true, 0, 159).is_err());
        let mut state = State::default();
        assert!(state.observe(
            403,
            "X-RateLimit-Remaining: 0\nX-RateLimit-Reset: 500",
            b"{}",
            100
        ));
        assert_eq!(state.blocked_until, 500);
    }

    #[test]
    fn delayed_measurements_cannot_roll_back_a_newer_bucket_window() {
        let mut state = State::default();
        state.observe_budget("core", 100, 300);
        state.observe_budget("core", 0, 200);
        assert_eq!(state.buckets["core"].reset, 300);
        assert_eq!(state.buckets["core"].remaining, 100);
        assert_eq!(rest_category("/search/code?q=abc"), "code_search");
        assert_eq!(rest_category("search/issues?q=abc"), "search");
    }
}
