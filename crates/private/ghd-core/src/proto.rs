//! Client↔daemon wire messages (protobuf bodies over loopback HTTP).
//!
//! Field tags are stable: retire a tag with `reserved`, never reuse it.

/// `POST /v1/pr/view`.
#[derive(Clone, PartialEq, prost::Message)]
pub struct PrViewRequest {
    #[prost(string, tag = "1")]
    pub owner: String,
    #[prost(string, tag = "2")]
    pub repo: String,
    #[prost(uint64, tag = "3")]
    pub number: u64,
    /// `gh pr view --json` field names.
    #[prost(string, repeated, tag = "4")]
    pub fields: Vec<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct PrViewResponse {
    /// The gh-shaped JSON document, ready to print.
    #[prost(bytes = "vec", tag = "1")]
    pub json: Vec<u8>,
    /// How it was answered: `fresh`, `revalidated` (ETag 304) or `fetched`.
    #[prost(string, tag = "2")]
    pub source: String,
    /// Non-empty when the read failed.
    #[prost(string, tag = "3")]
    pub error: String,
}

/// `POST /v1/graphql`: a `gh api graphql` call.
#[derive(Clone, PartialEq, prost::Message)]
pub struct GraphqlRequest {
    #[prost(string, tag = "1")]
    pub query: String,
    #[prost(message, repeated, tag = "2")]
    pub variables: Vec<GraphqlVariable>,
}

/// One `-f name=value` (raw string) or `-F name=value` (typed) argument.
#[derive(Clone, PartialEq, prost::Message)]
pub struct GraphqlVariable {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub value: String,
    /// `-F`: `true`/`false`/`null`/integers become JSON scalars, as with gh.
    #[prost(bool, tag = "3")]
    pub typed: bool,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct GraphqlResponse {
    /// GitHub's response body, as `gh api graphql` prints it.
    #[prost(bytes = "vec", tag = "1")]
    pub json: Vec<u8>,
    /// `fresh` (an identical query answered within the fresh window, or
    /// waiting on one in flight) or `fetched`.
    #[prost(string, tag = "2")]
    pub source: String,
    /// Non-empty when the call failed or the body carried `errors`.
    #[prost(string, tag = "3")]
    pub error: String,
}
