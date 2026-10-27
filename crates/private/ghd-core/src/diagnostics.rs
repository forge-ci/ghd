//! Conservative offline replay: exact request hashes, never inferred resources.
use kernal_api::json::{self, Value};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read, Write},
};
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid schema-v1 ledger input")
}
fn number(value: Option<&Value>) -> io::Result<u64> {
    match value {
        Some(Value::Unsigned(v)) => Ok(*v),
        Some(Value::Signed(v)) if *v >= 0 => u64::try_from(*v).map_err(|_| invalid()),
        _ => Err(invalid()),
    }
}
pub fn redact(args: &[String]) -> Vec<String> {
    args.iter()
        .take(128)
        .map(|arg| match arg.as_str() {
            "--json" | "--repo" | "-R" | "--limit" | "--header" | "-H" | "--field" | "-F"
            | "--raw-field" | "-f" | "--input" | "--hostname" => arg.clone(),
            _ => "[redacted]".into(),
        })
        .collect()
}
pub fn replay(bytes: &[u8], ttl_ms: u64) -> io::Result<Value> {
    if bytes.len() > 16 * 1024 * 1024 || ttl_ms > 86400000 {
        return Err(invalid());
    }
    let Value::Object(doc) = json::parse(bytes).map_err(|_| invalid())? else {
        return Err(invalid());
    };
    if number(doc.get("schema"))? != 1 {
        return Err(invalid());
    }
    let Some(Value::Array(entries)) = doc.get("entries") else {
        return Err(invalid());
    };
    if entries.len() > 4096 {
        return Err(invalid());
    }
    let mut cache = crate::retention::Cache::new(4096, 1024 * 1024);
    let (mut hits, mut eligible, mut observed, mut unknown, mut last) =
        (0u64, 0u64, 0u64, 0u64, 0u64);
    for entry in entries {
        let Value::Object(entry) = entry else {
            return Err(invalid());
        };
        let at = number(entry.get("at_ms"))?;
        if at < last {
            return Err(invalid());
        }
        last = at;
        let Some(Value::String(key)) = entry.get("key") else {
            return Err(invalid());
        };
        if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let Some(Value::String(source)) = entry.get("source") else {
            return Err(invalid());
        };
        if !matches!(
            source.as_str(),
            "hit" | "miss" | "coalesced" | "304" | "refused" | "stale" | "write"
        ) {
            return Err(invalid());
        }
        match entry.get("upstream_dispatches") {
            Some(Value::Null) => unknown += 1,
            value => observed = observed.checked_add(number(value)?).ok_or_else(invalid)?,
        }
        let Some(Value::String(kind)) = entry.get("operation_kind") else {
            return Err(invalid());
        };
        let Some(Value::Bool(cache_eligible)) = entry.get("cache_eligible") else {
            return Err(invalid());
        };
        if !matches!(kind.as_str(), "read" | "write" | "unknown")
            || (*cache_eligible && kind != "read")
        {
            return Err(invalid());
        }
        if kind != "read" {
            cache.clear();
        }
        if !cache_eligible || matches!(source.as_str(), "refused" | "stale" | "write") {
            continue;
        }
        eligible += 1;
        if cache.get(key).is_some_and(|expires| at < *expires) {
            hits += 1;
        } else {
            cache.insert(key.clone(), at.saturating_add(ttl_ms), 128);
        }
    }
    let n = |v| Value::Unsigned(v);
    Ok(Value::Object(BTreeMap::from([
        ("schema".into(), n(1)), ("network_free".into(), Value::Bool(true)),
        ("input_blake3".into(), Value::String(kernal_api::hash::blake3_bytes(bytes).to_hex())),
        ("ttl_ms".into(), n(ttl_ms)), ("eligible_requests".into(), n(eligible)),
        ("excluded_requests".into(), n(entries.len() as u64 - eligible)),
        ("modeled_exact_key_hits".into(), n(hits)),
        ("modeled_logical_origin_reads".into(), n(eligible - hits)),
        ("observed_known_dispatches".into(), n(observed)),
        ("unknown_dispatch_records".into(), n(unknown)),
        ("observed_total_dispatches".into(), if unknown == 0 { n(observed) } else { Value::Null }),
        ("provider_points_saved".into(), Value::Null),
        ("assumptions".into(), Value::String("Model only: exact request hashes; completion-time order; supplied fixed TTL; no resource sharing, origin policy, unrecorded write epochs or provider point model; recorded writes clear modeled retention; bounded exported window may omit prior work.".into())),
    ])))
}
pub fn run(args: &[&str]) -> io::Result<()> {
    let [path, "--ttl", ttl] = args else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "replay FILE --ttl MS",
        ));
    };
    let ttl = ttl.parse().map_err(|_| invalid())?;
    if !std::fs::metadata(path)?.is_file() {
        return Err(invalid());
    }
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    let result = replay(&bytes, ttl)?;
    io::stdout()
        .lock()
        .write_all(&json::encode(&result, json::Layout::Pretty).map_err(io::Error::other)?)?;
    println!();
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_redacts_headers_variables_tokens_and_body_file_inputs() {
        let args = [
            "api",
            "-H",
            "Authorization: token secret",
            "-f",
            "query=private",
            "--input",
            "@private.json",
            "--token=secret",
        ]
        .map(String::from);
        let capture = redact(&args).join(" ");
        for secret in ["secret", "private", "Authorization", "query="] {
            assert!(!capture.contains(secret));
        }
    }
    #[test]
    fn replay_rejects_unknown_sources_and_oversized_policy() {
        assert!(replay(br#"{"schema":1,"entries":[]}"#, 86400001).is_err());
        assert!(replay(br#"{"schema":2,"entries":[]}"#, 10).is_err());
        assert!(replay(b"not json", 0).is_err());
    }
    #[test]
    fn opaque_different_keys_never_invent_resource_sharing() {
        let row = |key: &str| {
            format!(
                r#"{{"at_ms":0,"key":"{key}","source":"miss","operation_kind":"read","cache_eligible":true,"upstream_dispatches":null}}"#
            )
        };
        let input = format!(
            r#"{{"schema":1,"entries":[{},{}]}}"#,
            row(&"a".repeat(64)),
            row(&"b".repeat(64))
        );
        let result = replay(input.as_bytes(), 1000).expect("model");
        let Value::Object(result) = result else {
            panic!("object");
        };
        assert_eq!(
            result.get("modeled_exact_key_hits"),
            Some(&Value::Unsigned(0))
        );
        assert_eq!(result.get("observed_total_dispatches"), Some(&Value::Null));
        assert_eq!(result.get("provider_points_saved"), Some(&Value::Null));
        assert!(replay(input.replace("miss", "invented").as_bytes(), 1000).is_err());
    }
}
