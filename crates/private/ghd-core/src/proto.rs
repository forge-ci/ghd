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
    /// Explicit hostname; empty uses GH_HOST or github.com.
    #[prost(string, tag = "5")]
    pub hostname: String,
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
    #[prost(string, tag = "3")]
    pub hostname: String,
    #[prost(string, tag = "4")]
    pub operation_name: String,
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

/// `POST /v1/read/gh`: a classified headless read through a real-gh backend.
#[derive(Clone, PartialEq, prost::Message)]
pub struct GhReadRequest {
    #[prost(string, repeated, tag = "1")]
    pub args: Vec<String>,
    #[prost(string, tag = "2")]
    pub cwd: String,
    #[prost(string, tag = "3")]
    pub program: String,
    #[prost(string, tag = "4")]
    pub program_stamp: String,
    #[prost(string, tag = "5")]
    pub default_repo: String,
    #[prost(string, tag = "6")]
    pub worktree_stamp: String,
    #[prost(message, repeated, tag = "7")]
    pub environment: Vec<EnvironmentVariable>,
    #[prost(string, tag = "8")]
    pub hostname: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct EnvironmentVariable {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct GhReadResponse {
    #[prost(bytes = "vec", tag = "1")]
    pub stdout: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub stderr: Vec<u8>,
    #[prost(int32, tag = "3")]
    pub exit_code: i32,
    #[prost(string, tag = "4")]
    pub source: String,
    #[prost(string, tag = "5")]
    pub error: String,
    /// Reaching a caller's limit is not proof of collection completeness.
    #[prost(bool, tag = "6")]
    pub limit_reached: bool,
}

/// A first-class repository planning/authority snapshot; versioned route.
#[derive(Clone, PartialEq, prost::Message)]
pub struct SnapshotRequest {
    #[prost(message, optional, tag = "1")]
    pub backend: Option<GhReadRequest>,
    #[prost(string, tag = "2")]
    pub owner: String,
    #[prost(string, tag = "3")]
    pub repo: String,
    #[prost(uint32, tag = "4")]
    pub limit: u32,
    #[prost(bool, tag = "5")]
    pub fresh: bool,
    #[prost(bool, tag = "6")]
    pub allow_stale: bool,
    #[prost(uint32, tag = "7")]
    pub version: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct SnapshotResponse {
    #[prost(bytes = "vec", tag = "1")]
    pub json: Vec<u8>,
    #[prost(string, tag = "2")]
    pub source: String,
    #[prost(uint64, optional, tag = "3")]
    pub age_ms: Option<u64>,
    #[prost(bool, tag = "4")]
    pub stale: bool,
    #[prost(bool, tag = "5")]
    pub truncated: bool,
    #[prost(int32, tag = "6")]
    pub exit_code: i32,
    #[prost(string, tag = "7")]
    pub error: String,
}
