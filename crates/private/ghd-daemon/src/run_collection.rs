//! Actions collection translation for fields supplied by the collection itself.
//! Workflow names/jobs require separate metadata and remain native gh reads.

use ghd_core::resource::ReadPlan;
use kernal_api::json::{self, Layout, Value};
use std::collections::BTreeMap;
use std::io;

const FIELDS: &[(&str, &str)] = &[
    ("databaseId", "id"),
    ("number", "run_number"),
    ("attempt", "run_attempt"),
    ("workflowDatabaseId", "workflow_id"),
    ("name", "name"),
    ("displayTitle", "display_title"),
    ("headBranch", "head_branch"),
    ("headSha", "head_sha"),
    ("event", "event"),
    ("status", "status"),
    ("conclusion", "conclusion"),
    ("createdAt", "created_at"),
    ("startedAt", "run_started_at"),
    ("updatedAt", "updated_at"),
    ("url", "html_url"),
];

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            use std::fmt::Write;
            // Writing a String is infallible.
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// None means native gh must handle the shape, including richer retained fields.
pub fn path(plan: &ReadPlan, retained: &[String]) -> Option<String> {
    if !plan.resource
        || plan.kind != "run"
        || plan.action != "list"
        || plan
            .fetch_fields
            .iter()
            .chain(retained)
            .any(|f| !FIELDS.iter().any(|(name, _)| name == f))
        || plan.fetch_args.iter().any(|arg| arg == "--all")
    {
        return None;
    }
    let mut query = String::from("per_page=100&exclude_pull_requests=true");
    for (flag, value) in &plan.filters {
        let name = match flag.as_str() {
            "--branch" => "branch",
            "--commit" => "head_sha",
            "--created" => "created",
            "--event" => "event",
            "--status" => "status",
            "--user" => "actor",
            _ => return None,
        };
        query.push_str(&format!("&{name}={}", encode(value)));
    }
    Some(format!(
        "/repos/{}/actions/runs?{query}",
        plan.repo.as_ref()?
    ))
}

/// Decode gh api --include without losing the retained body's validator.
/// Both HTTP/1 CRLF and gh's normalized LF output are accepted.
pub fn included_response(output: &[u8]) -> io::Result<(u16, Option<String>, &[u8])> {
    let boundaries = [
        output
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|end| (end, 4)),
        output
            .windows(2)
            .position(|w| w == b"\n\n")
            .map(|end| (end, 2)),
    ];
    let (end, separator) = boundaries
        .into_iter()
        .flatten()
        .min_by_key(|(end, _)| *end)
        .ok_or_else(|| invalid("gh included response has no header boundary"))?;
    if end > 64 * 1024 {
        return Err(invalid("gh response headers exceed limit"));
    }
    let headers = std::str::from_utf8(&output[..end]).map_err(io::Error::other)?;
    let mut lines = headers.lines();
    let status = lines.next().ok_or_else(|| invalid("missing HTTP status"))?;
    let mut words = status.split_whitespace();
    if !words.next().is_some_and(|word| word.starts_with("HTTP/")) {
        return Err(invalid("invalid HTTP status"));
    }
    let status = words
        .next()
        .and_then(|word| word.parse::<u16>().ok())
        .filter(|code| (100..600).contains(code))
        .ok_or_else(|| invalid("invalid HTTP status code"))?;
    let mut etag = None;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| invalid("invalid HTTP header"))?;
        if name.eq_ignore_ascii_case("etag") {
            let value = value.trim();
            if etag.is_some()
                || value.is_empty()
                || value.len() > 1024
                || value.bytes().any(|byte| byte < 32 || byte == 127)
            {
                return Err(invalid("invalid collection ETag"));
            }
            etag = Some(value.into());
        }
    }
    Ok((status, etag, &output[end + separator..]))
}

/// Match gh's Run.ExportData, including Go zero values for nullable fields.
pub fn translate(body: &[u8]) -> io::Result<Vec<u8>> {
    let Value::Object(root) = json::parse(body).map_err(io::Error::other)? else {
        return Err(invalid("Actions collection must be an object"));
    };
    let Some(Value::Array(rows)) = root.get("workflow_runs") else {
        return Err(invalid("Actions collection is missing workflow_runs"));
    };
    let mut translated = Vec::new();
    for row in rows.iter().take(100) {
        let Value::Object(row) = row else {
            return Err(invalid("Actions run must be an object"));
        };
        let mut out = BTreeMap::new();
        for (name, wire) in FIELDS {
            let value = row.get(*wire).unwrap_or(&Value::Null);
            let numeric = matches!(
                *name,
                "databaseId" | "number" | "attempt" | "workflowDatabaseId"
            );
            let value = match (numeric, value) {
                (true, Value::Signed(n)) if *n >= 0 => Value::Signed(*n),
                (true, Value::Unsigned(n)) if i64::try_from(*n).is_ok() => Value::Unsigned(*n),
                (true, Value::Null) => Value::Signed(0),
                (false, Value::String(s)) => Value::String(s.clone()),
                (false, Value::Null) => Value::String(if name.ends_with("At") {
                    "0001-01-01T00:00:00Z".into()
                } else {
                    String::new()
                }),
                _ => return Err(invalid("Actions run field has an incompatible type")),
            };
            out.insert((*name).into(), value);
        }
        translated.push(Value::Object(out));
    }
    json::encode(&Value::Array(translated), Layout::Compact).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn included_headers_bind_status_validator_and_body() {
        for output in [
            b"HTTP/2.0 200 OK\nEtag: \"v1\"\nContent-Type: application/json\n\n{}".as_slice(),
            b"HTTP/1.1 200 OK\r\nETag: \"v1\"\r\n\r\n{}".as_slice(),
        ] {
            let (status, etag, body) = included_response(output).expect("decode");
            assert_eq!(status, 200);
            assert_eq!(etag.as_deref(), Some("\"v1\""));
            assert_eq!(body, b"{}");
        }
        let (_, _, body) =
            included_response(b"HTTP/2.0 200 OK\n\n{\r\n\r\n}").expect("earliest boundary");
        assert_eq!(body, b"{\r\n\r\n}");
        assert!(included_response(b"{}\n").is_err());
        assert!(included_response(b"HTTP/1.1 304 Not Modified\nETag: a\nETag: b\n\n").is_err());
    }

    #[test]
    fn collection_nulls_attempts_and_distinct_workflow_names() {
        let output = translate(br#"{"workflow_runs":[{"id":42,"run_attempt":2,"name":"run name","conclusion":null,"run_started_at":null}]}"#).expect("translate");
        let Value::Array(rows) = json::parse(&output).expect("JSON") else {
            panic!("array")
        };
        let Value::Object(row) = &rows[0] else {
            panic!("object")
        };
        assert_eq!(row["attempt"], Value::Signed(2));
        assert_eq!(row["conclusion"], Value::String(String::new()));
        assert_eq!(
            row["startedAt"],
            Value::String("0001-01-01T00:00:00Z".into())
        );
        assert!(!row.contains_key("workflowName"));
        assert!(translate(br#"{"workflow_runs":[{"run_attempt":"2"}]}"#).is_err());
    }

    #[test]
    fn filters_are_encoded_and_metadata_dependencies_disable_collection_shortcut() {
        let args = [
            "run",
            "list",
            "-R",
            "o/r",
            "--branch",
            "feature/a&b",
            "--json",
            "attempt",
        ];
        let plan = ghd_core::resource::prepare(&args.map(String::from), "github.com", None)
            .expect("prepare")
            .expect("bound");
        assert!(
            path(&plan, &[])
                .expect("collection")
                .ends_with("branch=feature%2Fa%26b")
        );
        assert!(path(&plan, &["workflowName".into()]).is_none());
    }
}
