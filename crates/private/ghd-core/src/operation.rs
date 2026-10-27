//! Parse GraphQL documents before deciding whether they can enter a cache.

use graphql_parser::query::{Definition, OperationDefinition};
use std::collections::BTreeSet;
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Read,
    Write,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

pub fn classify(document: &str, operation_name: Option<&str>) -> io::Result<Operation> {
    let parsed = graphql_parser::parse_query::<&str>(document)
        .map_err(|_| invalid("invalid GraphQL document"))?;
    let mut operations = Vec::new();
    let mut names = BTreeSet::new();
    for definition in parsed.definitions {
        let Definition::Operation(operation) = definition else {
            continue;
        };
        let (name, kind) = match operation {
            OperationDefinition::SelectionSet(_) => (None, Operation::Read),
            OperationDefinition::Query(query) => (query.name, Operation::Read),
            OperationDefinition::Mutation(mutation) => (mutation.name, Operation::Write),
            OperationDefinition::Subscription(_) => {
                return Err(invalid("GraphQL subscriptions are unsupported"));
            }
        };
        if let Some(name) = name
            && !names.insert(name)
        {
            return Err(invalid("duplicate GraphQL operation name"));
        }
        operations.push((name, kind));
    }
    if operations.is_empty() {
        return Err(invalid("GraphQL document contains no operation"));
    }
    if operations.len() > 1 && operations.iter().any(|(name, _)| name.is_none()) {
        return Err(invalid(
            "anonymous GraphQL operations cannot share a document",
        ));
    }
    if let Some(selected) = operation_name {
        return operations
            .iter()
            .find(|(name, _)| *name == Some(selected))
            .map(|(_, kind)| *kind)
            .ok_or_else(|| invalid("unknown GraphQL operationName"));
    }
    if operations.len() != 1 {
        return Err(invalid("multiple GraphQL operations require operationName"));
    }
    Ok(operations[0].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragments_comments_strings_and_bom_do_not_hide_writes() {
        for document in [
            "fragment Fields on User { id } mutation Write { updateUser { ...Fields } }",
            "\u{feff}# comment\n mutation { updateUser { id } }",
            "mutation M($s: String = \"query # not a comment\") { updateUser(name: $s) { id } }",
        ] {
            assert_eq!(classify(document, None).expect("write"), Operation::Write);
        }
        assert_eq!(
            classify("{ viewer { login } }", None).expect("read"),
            Operation::Read
        );
    }

    #[test]
    fn operation_name_selects_exactly_the_executed_operation() {
        let doc = "query Read { viewer { login } } mutation Write { updateUser { id } }";
        assert!(classify(doc, None).is_err());
        assert_eq!(classify(doc, Some("Read")).expect("read"), Operation::Read);
        assert_eq!(
            classify(doc, Some("Write")).expect("write"),
            Operation::Write
        );
        assert!(classify(doc, Some("Absent")).is_err());
    }

    #[test]
    fn malformed_ambiguous_or_unsupported_documents_fail_closed() {
        for document in [
            "",
            "query {",
            "{ x } mutation M { y }",
            "fragment F on User { id }",
            "query Same { x } query Same { y }",
            "subscription { event { id } }",
            "query Read { x } trailing garbage",
        ] {
            assert!(classify(document, None).is_err(), "{document:?}");
        }
    }
}
