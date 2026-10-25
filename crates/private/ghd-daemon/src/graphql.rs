//! `POST /v1/graphql`: `gh api graphql` through the daemon.
//!
//! Identical concurrent queries make one upstream call (one lock per
//! query+variables), and an answer is served `fresh` for the fresh window
//! (`GHD_FRESH_MS`, 2 s), which is what agents' re-check loops hit. A
//! mutation always goes upstream and empties the query cache, so no read
//! issued after a write sees a pre-write answer.

use ghd_core::proto::GraphqlVariable;
use kernal_api::json::Value;
use std::collections::BTreeMap;

/// The JSON request body GitHub's `/graphql` takes.
pub fn body(query: &str, variables: &[GraphqlVariable]) -> Result<Vec<u8>, String> {
    let _ = (query, variables);
    Err("not implemented".into())
}

/// Whether the document is a mutation (never cached).
pub fn is_mutation(query: &str) -> bool {
    let _ = query;
    false
}

/// The message of the response's `errors`, if it has any.
pub fn errors(json: &[u8]) -> Option<String> {
    let _ = json;
    None
}

fn _unused(_: BTreeMap<String, Value>) {}

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
        let Value::Object(top) = v else { panic!("object") };
        assert_eq!(top["query"], Value::String("q".into()));
        let Value::Object(vars) = &top["variables"] else { panic!("variables") };
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
