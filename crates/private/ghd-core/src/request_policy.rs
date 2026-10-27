//! Explicit per-request controls; thread-local scopes never alter the environment.
use std::{cell::Cell, io, time::Duration};

#[derive(Clone, Copy, Default, Debug)]
pub struct Policy {
    pub bypass: bool,
    pub ttl_ms: Option<u64>,
}
thread_local! { static CURRENT: Cell<Policy> = Cell::new(Policy::default()); }
struct Reset(Policy);
impl Drop for Reset {
    fn drop(&mut self) {
        CURRENT.set(self.0);
    }
}
pub fn scope<T>(policy: Policy, operation: impl FnOnce() -> T) -> T {
    let _reset = Reset(CURRENT.replace(policy));
    operation()
}
pub fn current() -> Policy {
    CURRENT.get()
}
impl Policy {
    pub fn active(self) -> bool {
        self.bypass || self.ttl_ms.is_some()
    }
    pub fn ttl(self, default: Duration) -> Duration {
        if self.bypass {
            Duration::ZERO
        } else {
            self.ttl_ms
                .map_or(default, Duration::from_millis)
                .min(default)
        }
    }
    pub fn header(self) -> String {
        format!(
            "1;{};{}",
            u8::from(self.bypass),
            self.ttl_ms.map_or_else(String::new, |v| v.to_string())
        )
    }
    pub fn parse(value: &str) -> io::Result<Self> {
        let parts: Vec<_> = value.split(';').collect();
        let invalid =
            || io::Error::new(io::ErrorKind::InvalidInput, "invalid request cache policy");
        if parts.len() != 3 || parts[0] != "1" || !matches!(parts[1], "0" | "1") {
            return Err(invalid());
        }
        Ok(Self {
            bypass: parts[1] == "1",
            ttl_ms: if parts[2].is_empty() {
                None
            } else {
                Some(parts[2].parse().map_err(|_| invalid())?)
            },
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scoped_policy_restores_and_never_extends_origin_ttl() {
        assert!(!current().active());
        scope(
            Policy {
                bypass: true,
                ttl_ms: None,
            },
            || assert!(current().bypass),
        );
        assert!(!current().active());
        assert_eq!(
            Policy {
                bypass: false,
                ttl_ms: Some(500)
            }
            .ttl(Duration::from_millis(100)),
            Duration::from_millis(100)
        );
        assert!(Policy::parse("1;1;bad").is_err());
        assert!(Policy::parse("2;0;").is_err());
    }
}
