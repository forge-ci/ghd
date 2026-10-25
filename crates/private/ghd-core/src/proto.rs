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
