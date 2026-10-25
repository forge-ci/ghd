//! `POST /v1/graphql`: `gh api graphql` through the daemon.
//!
//! Identical concurrent queries make one upstream call (one lock per
//! query+variables), and an answer is served `fresh` for the fresh window
//! (`GHD_FRESH_MS`, 2 s), which is what agents' re-check loops hit. A
//! mutation always goes upstream and empties the query cache, so no read
//! issued after a write sees a pre-write answer.

use ghd_core::proto::GraphqlVariable;
use kernal_api::json::{self, Layout, Value};
use std::collections::BTreeMap;

/// The JSON request body GitHub's `/graphql` takes. Also the cache key:
/// the same query with the same variables encodes to the same bytes.
pub fn body(query: &str, variables: &[GraphqlVariable]) -> Result<Vec<u8>, String> {
    let mut doc = BTreeMap::new();
    doc.insert("query".to_string(), Value::String(query.to_string()));
    if !variables.is_empty() {
        let vars = variables
            .iter()
            .map(|v| (v.name.clone(), value(v)))
            .collect();
        doc.insert("variables".to_string(), Value::Object(vars));
    }
    json::encode(&Value::Object(doc), Layout::Compact).map_err(|e| e.to_string())
}

/// gh's `-F` typing: `true`, `false`, `null` and integers are JSON
/// scalars; anything else (and every `-f`) is a string.
fn value(v: &GraphqlVariable) -> Value {
    if !v.typed {
        return Value::String(v.value.clone());
    }
    match v.value.as_str() {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        other => other
            .parse::<i64>()
            .map_or_else(|_| Value::String(other.to_string()), Value::Signed),
    }
}

/// Whether the document is a mutation (never cached).
pub fn is_mutation(query: &str) -> bool {
    let code = query
        .lines()
        .map(|line| line.split('#').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ");
    code.trim_start().starts_with("mutation")
}

/// The message of the response's `errors`, if it has any.
pub fn errors(json_body: &[u8]) -> Option<String> {
    let Ok(Value::Object(top)) = json::parse(json_body) else {
        return Some(String::from_utf8_lossy(json_body).into_owned());
    };
    let Value::Array(errors) = top.get("errors")? else {
        return None;
    };
    let messages: Vec<String> = errors
        .iter()
        .map(|e| match e {
            Value::Object(m) => match m.get("message") {
                Some(Value::String(s)) => s.clone(),
                _ => "error".to_string(),
            },
            _ => "error".to_string(),
        })
        .collect();
    (!messages.is_empty()).then(|| messages.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernal_api::json;

    fn var(name: &str, value: &str, typed: bool) -> GraphqlVariable {
        GraphqlVariable {
            name: name.into(),
            value: value.into(),
            typed,
        }
    }

    #[test]
    fn typed_fields_become_json_scalars_like_gh() {
        let b = body(
            "q",
            &[
                var("n", "3", true),
                var("s", "3", false),
                var("t", "true", true),
                var("z", "null", true),
                var("w", "word", true),
            ],
        )
        .expect("encodes");
        let v = json::parse(&b).expect("json");
        let Value::Object(top) = v else {
            panic!("object")
        };
        assert_eq!(top["query"], Value::String("q".into()));
        let Value::Object(vars) = &top["variables"] else {
            panic!("variables")
        };
        assert!(matches!(vars["n"], Value::Signed(3) | Value::Unsigned(3)));
        assert_eq!(vars["s"], Value::String("3".into()));
        assert_eq!(vars["t"], Value::Bool(true));
        assert_eq!(vars["z"], Value::Null);
        assert_eq!(vars["w"], Value::String("word".into()));
    }

    #[test]
    fn no_variables_sends_no_variables_member() {
        let b = body("{viewer{login}}", &[]).expect("encodes");
        assert!(!String::from_utf8_lossy(&b).contains("variables"));
    }

    #[test]
    fn mutations_are_recognized_past_comments_and_whitespace() {
        assert!(is_mutation("mutation { x }"));
        assert!(is_mutation("  # note\n mutation M($id:ID!){ x }"));
        assert!(!is_mutation("query { mutationQueue }"));
        assert!(!is_mutation("{ repository { id } }"));
    }

    #[test]
    fn errors_are_reported_and_clean_bodies_are_not() {
        let msg = errors(br#"{"errors":[{"message":"API rate limit exceeded"}]}"#);
        assert_eq!(msg.as_deref(), Some("API rate limit exceeded"));
        assert_eq!(errors(br#"{"data":{"x":1}}"#), None);
    }
}
