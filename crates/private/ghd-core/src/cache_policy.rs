//! Explicit per-command freshness and narrowly scoped completed-run policy.

use crate::resource::ReadPlan;
use kernal_api::json::{self, Value};
use std::{collections::BTreeMap, io, time::Duration};

#[derive(Default)]
pub struct Policy {
    commands: BTreeMap<String, Duration>,
    settled: Option<Duration>,
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "invalid GHD_CACHE_TTLS or GHD_CACHE_SETTLED_MS",
    )
}

impl Policy {
    pub fn from_environment() -> io::Result<Self> {
        Self::parse(
            crate::config::value("GHD_CACHE_TTLS")?.as_deref(),
            crate::config::value("GHD_CACHE_SETTLED_MS")?.as_deref(),
        )
    }

    pub fn parse(commands: Option<&str>, settled: Option<&str>) -> io::Result<Self> {
        let mut out = Self::default();
        if let Some(raw) = commands {
            if raw.len() > 4096 {
                return Err(invalid());
            }
            let Value::Object(values) = json::parse(raw.as_bytes()).map_err(|_| invalid())? else {
                return Err(invalid());
            };
            if values.len() > 32 {
                return Err(invalid());
            }
            for (key, value) in values {
                if key.len() > 64
                    || key.split(' ').count() != 2
                    || !key
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b == b' ' || b == b'-')
                {
                    return Err(invalid());
                }
                let ms = match value {
                    Value::Unsigned(value) => value,
                    Value::Signed(value) => u64::try_from(value).map_err(|_| invalid())?,
                    _ => return Err(invalid()),
                };
                out.commands.insert(key, Duration::from_millis(ms));
            }
        }
        out.settled = settled
            .map(|raw| {
                raw.parse::<u64>()
                    .map(Duration::from_millis)
                    .map_err(|_| invalid())
            })
            .transpose()?;
        Ok(out)
    }

    pub fn command(&self, kind: &str, action: &str, default: Duration) -> Duration {
        self.commands
            .get(&format!("{kind} {action}"))
            .copied()
            .unwrap_or(default)
    }

    pub fn resource(&self, plan: &ReadPlan, body: &[u8], default: Duration) -> Duration {
        let ordinary = self.command(&plan.kind, &plan.action, default);
        let Some(settled) = self.settled else {
            return ordinary;
        };
        if plan.kind != "run"
            || plan.action != "view"
            || !plan.resource
            || !plan.filters.contains_key("--attempt")
            || plan.fields.is_empty()
            || !plan.fields.iter().all(|field| {
                matches!(
                    field.as_str(),
                    "attempt"
                        | "conclusion"
                        | "databaseId"
                        | "headSha"
                        | "number"
                        | "startedAt"
                        | "status"
                        | "url"
                )
            })
        {
            return ordinary;
        }
        let Ok(Value::Object(value)) = json::parse(body) else {
            return ordinary;
        };
        if value.get("status") == Some(&Value::String("completed".into())) {
            settled
        } else {
            ordinary
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_short_and_invalid_configuration_is_explicit() {
        let default = Duration::from_secs(2);
        let policy =
            Policy::parse(Some(r#"{"issue view":0,"run list":500}"#), None).expect("policy");
        assert_eq!(policy.command("issue", "view", default), Duration::ZERO);
        assert_eq!(
            policy.command("run", "list", default),
            Duration::from_millis(500)
        );
        assert_eq!(policy.command("pr", "view", default), default);
        for value in [
            "no",
            "[]",
            r#"{"issue view":-1}"#,
            r#"{"issue view":"500"}"#,
        ] {
            assert!(Policy::parse(Some(value), None).is_err());
        }
    }
    #[test]
    fn settled_policy_requires_explicit_attempt_and_stable_projection() {
        let policy = Policy::parse(None, Some("60000")).expect("policy");
        let default = Duration::from_secs(2);
        let prepare = |fields: &str, attempt: bool| {
            let mut args = vec![
                "run".into(),
                "view".into(),
                "42".into(),
                "-R".into(),
                "o/r".into(),
                "--json".into(),
                fields.into(),
            ];
            if attempt {
                args.extend(["--attempt".into(), "1".into()]);
            }
            crate::resource::prepare(&args, "github.com", None)
                .expect("parse")
                .expect("plan")
        };
        let complete = br#"{"status":"completed","headSha":"abc"}"#;
        assert_eq!(
            policy.resource(&prepare("headSha,status", true), complete, default),
            Duration::from_secs(60)
        );
        assert_eq!(
            policy.resource(&prepare("headSha,status", false), complete, default),
            default
        );
        assert_eq!(
            policy.resource(&prepare("workflowName,status", true), complete, default),
            default
        );
        assert_eq!(
            policy.resource(
                &prepare("headSha,status", true),
                br#"{"status":"in_progress"}"#,
                default
            ),
            default
        );
    }
}
