//! `gh pr view --json` names ghd can answer: plain GraphQL scalars with the
//! same name and the same value shape gh prints. The daemon builds its
//! selection only from this list, so it is also the injection allowlist.

pub const PR_FIELDS: &[&str] = &[
    "additions",
    "baseRefName",
    "body",
    "changedFiles",
    "closed",
    "closedAt",
    "createdAt",
    "deletions",
    "headRefName",
    "headRefOid",
    "id",
    "isDraft",
    "mergeStateStatus",
    "mergeable",
    "mergedAt",
    "number",
    "state",
    "title",
    "updatedAt",
    "url",
];

/// An owner or repository name safe to splice into a GraphQL string literal.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_could_inject_graphql_are_rejected() {
        assert!(valid_name("FastLED"));
        assert!(valid_name("ci.yml"));
        assert!(!valid_name("a\"){x}"));
        assert!(!valid_name(""));
    }

    #[test]
    fn field_list_is_sorted_and_unique() {
        assert!(PR_FIELDS.windows(2).all(|w| w[0] < w[1]));
    }
}
