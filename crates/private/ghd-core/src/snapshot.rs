//! Repository snapshots share one documented GraphQL selection and schema.

use crate::{
    client, compat, identity,
    proto::{SnapshotRequest, SnapshotResponse},
    reads,
};
use kernal_api::json::{self, Layout, Value};
use prost::Message;
use std::{
    collections::BTreeMap,
    io::{self, Write},
};

const QUERY: &str = r"query GhdSnapshotV1($owner:String!,$name:String!,$limit:Int!) {
 repository(owner:$owner,name:$name) {
  nameWithOwner isPrivate
  defaultBranchRef { name target { oid ... on Commit { ...GhdSnapshotCommit } } }
  pullRequests(first:$limit,states:OPEN,orderBy:{field:UPDATED_AT,direction:DESC}) {
   totalCount pageInfo { hasNextPage hasPreviousPage startCursor endCursor }
   nodes {
    number title state isDraft createdAt updatedAt mergeable mergeStateStatus reviewDecision
    headRefName headRefOid baseRefName baseRefOid additions deletions changedFiles
    files(first:100) { totalCount pageInfo { hasNextPage hasPreviousPage startCursor endCursor } nodes { path additions deletions } }
    reviews(last:100) { totalCount pageInfo { hasNextPage hasPreviousPage startCursor endCursor } nodes { state submittedAt author { login } commit { oid } } }
    commits(last:1) { totalCount pageInfo { hasNextPage hasPreviousPage startCursor endCursor } nodes { commit { ...GhdSnapshotCommit } } }
   }
  }
 }
}
fragment GhdSnapshotCommit on Commit {
 oid committedDate statusCheckRollup {
  state contexts(first:100) {
   pageInfo { hasNextPage hasPreviousPage startCursor endCursor }
   nodes {
    __typename
    ... on CheckRun { databaseId name status conclusion detailsUrl startedAt completedAt checkSuite { app { slug databaseId } workflowRun { databaseId event } } }
    ... on StatusContext { context state targetUrl }
   }
  }
 }
}";

#[derive(Clone, Debug)]
pub struct Spec {
    pub hostname: String,
    pub owner: String,
    pub repo: String,
    pub limit: u32,
    pub fresh: bool,
    pub allow_stale: bool,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn malformed(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub fn parse(args: &[String], default_repo: Option<&str>, default_host: &str) -> io::Result<Spec> {
    let mut repo = None;
    let mut host = None;
    let mut limit = 100;
    let mut fresh = false;
    let mut allow_stale = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let (name, inline) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(a, b)| (a, Some(b)));
        match name {
            "--fresh" if inline.is_none() && !fresh => fresh = true,
            "--allow-stale" if inline.is_none() && !allow_stale => allow_stale = true,
            "-R" | "--repo" | "--hostname" | "--limit" => {
                let value = inline
                    .or_else(|| it.next().map(String::as_str))
                    .ok_or_else(|| invalid("snapshot option needs a value"))?;
                match name {
                    "--limit" => {
                        limit = value
                            .parse()
                            .map_err(|_| invalid("snapshot limit must be 1..100"))?
                    }
                    "--hostname" => {
                        if host.replace(value.to_owned()).is_some() {
                            return Err(invalid("repeated snapshot hostname"));
                        }
                    }
                    _ => {
                        if repo.replace(value.to_owned()).is_some() {
                            return Err(invalid("repeated snapshot repository"));
                        }
                    }
                }
            }
            _ => return Err(invalid("unsupported or repeated snapshot option")),
        }
    }
    let selector = repo
        .as_deref()
        .or(default_repo)
        .ok_or_else(|| invalid("snapshot needs -R owner/repo or GH_REPO"))?;
    let (bound, owner, repo) = identity::bound_repository(selector, host.as_deref())?;
    let hostname = bound.map_or_else(|| identity::hostname(default_host), Ok)?;
    let spec = Spec {
        hostname,
        owner: owner.to_ascii_lowercase(),
        repo: repo.to_ascii_lowercase(),
        limit,
        fresh,
        allow_stale,
    };
    spec.validate()?;
    Ok(spec)
}

impl Spec {
    pub fn validate(&self) -> io::Result<()> {
        let (host, owner, repo) = identity::repository(&format!("{}/{}", self.owner, self.repo))?;
        if host.is_some()
            || owner != self.owner
            || repo != self.repo
            || !(1..=100).contains(&self.limit)
        {
            return Err(invalid("invalid snapshot repository or limit"));
        }
        if self.fresh && self.allow_stale {
            return Err(invalid("--fresh and --allow-stale are mutually exclusive"));
        }
        identity::hostname(&self.hostname)?;
        Ok(())
    }
    pub fn args(&self) -> Vec<String> {
        vec![
            "api".into(),
            "graphql".into(),
            "--hostname".into(),
            self.hostname.clone(),
            "--method".into(),
            "POST".into(),
            "--include".into(),
            "--header".into(),
            "Cache-Control: no-cache".into(),
            "--header".into(),
            "Pragma: no-cache".into(),
            "-f".into(),
            format!(
                "query={}",
                QUERY.split_whitespace().collect::<Vec<_>>().join(" ")
            ),
            "-f".into(),
            format!("owner={}", self.owner),
            "-f".into(),
            format!("name={}", self.repo),
            "-F".into(),
            format!("limit={}", self.limit),
        ]
    }
}

pub fn request(spec: &Spec) -> io::Result<SnapshotResponse> {
    spec.validate()?;
    if reads::uncached_context() {
        return Err(invalid(
            "debug/terminal context cannot be used for structured snapshots",
        ));
    }
    let backend = reads::backend_request(
        &[],
        spec.hostname.clone(),
        format!("{}/{}", spec.owner, spec.repo),
        false,
    )?;
    let request = SnapshotRequest {
        backend: Some(backend),
        owner: spec.owner.clone(),
        repo: spec.repo.clone(),
        limit: spec.limit,
        fresh: spec.fresh,
        allow_stale: spec.allow_stale,
        version: 1,
    };
    let (status, body) = client::call_host(
        Some(&spec.hostname),
        "POST",
        "/v2/snapshot",
        &request.encode_to_vec(),
    )?;
    if status != 200 {
        return Err(io::Error::other(format!(
            "daemon HTTP {status}: snapshots unsupported or refused; restart after upgrading"
        )));
    }
    SnapshotResponse::decode(body.as_slice()).map_err(|_| malformed("invalid snapshot reply"))
}

pub fn run(args: &[String]) -> io::Result<()> {
    let spec = parse(
        args,
        std::env::var("GH_REPO").ok().as_deref(),
        &std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".into()),
    )?;
    let response = request(&spec)?;
    if !response.json.is_empty() {
        io::stdout().lock().write_all(&response.json)?;
    }
    if !response.error.is_empty() {
        writeln!(io::stderr().lock(), "ghd snapshot: {}", response.error)?;
    }
    if response.exit_code == 0 {
        Ok(())
    } else {
        Err(compat::exit_error(response.exit_code))
    }
}

fn object(value: &Value) -> io::Result<&BTreeMap<String, Value>> {
    match value {
        Value::Object(value) => Ok(value),
        _ => Err(malformed("snapshot object is missing or invalid")),
    }
}
fn field<'a>(value: &'a BTreeMap<String, Value>, name: &str) -> io::Result<&'a Value> {
    value
        .get(name)
        .ok_or_else(|| malformed("snapshot selection is incomplete"))
}
fn connection<'a>(
    value: &'a BTreeMap<String, Value>,
    name: &str,
    truncated: &mut bool,
) -> io::Result<&'a [Value]> {
    let value = object(field(value, name)?)?;
    let Value::Array(nodes) = field(value, "nodes")? else {
        return Err(malformed("invalid snapshot nodes"));
    };
    let page = object(field(value, "pageInfo")?)?;
    for (flag, cursor) in [
        ("hasNextPage", "endCursor"),
        ("hasPreviousPage", "startCursor"),
    ] {
        let Value::Bool(more) = field(page, flag)? else {
            return Err(malformed("invalid snapshot pagination"));
        };
        if *more {
            *truncated = true;
            if !matches!(field(page,cursor)?, Value::String(value) if !value.is_empty()) {
                return Err(malformed("truncated snapshot has no cursor"));
            }
        }
    }
    if let Some(total) = value.get("totalCount") {
        let total = match total {
            Value::Unsigned(total) => *total,
            Value::Signed(total) if *total >= 0 => {
                u64::try_from(*total).map_err(io::Error::other)?
            }
            _ => return Err(malformed("invalid snapshot totalCount")),
        };
        if total > u64::try_from(nodes.len()).map_err(io::Error::other)? {
            *truncated = true;
        }
    }
    Ok(nodes)
}
fn commit(value: &BTreeMap<String, Value>, truncated: &mut bool) -> io::Result<()> {
    if !matches!(field(value, "oid")?, Value::String(value) if !value.is_empty()) {
        return Err(malformed("snapshot commit has no oid"));
    }
    let rollup = field(value, "statusCheckRollup")?;
    if !matches!(rollup, Value::Null) {
        connection(object(rollup)?, "contexts", truncated)?;
    }
    Ok(())
}

/// Preserve raw GitHub fields and require pagination evidence before claiming completeness.
pub fn repository(body: &[u8], expected: &str) -> io::Result<(Value, bool)> {
    if let Some(error) = crate::graphql::response_error(body) {
        return Err(io::Error::other(error));
    }
    let top = json::parse(body).map_err(io::Error::other)?;
    let top = object(&top)?;
    if top
        .get("errors")
        .is_some_and(|errors| !matches!(errors, Value::Array(_)))
    {
        return Err(malformed("invalid GraphQL errors"));
    }
    let repo = field(object(field(top, "data")?)?, "repository")?.clone();
    let fields = object(&repo)?;
    if !matches!(field(fields,"nameWithOwner")?, Value::String(name) if name.eq_ignore_ascii_case(expected))
    {
        return Err(malformed("snapshot repository does not match request"));
    }
    if !matches!(field(fields, "isPrivate")?, Value::Bool(_)) {
        return Err(malformed("snapshot privacy field is missing"));
    }
    let mut truncated = false;
    let branch = field(fields, "defaultBranchRef")?;
    if !matches!(branch, Value::Null) {
        let branch = object(branch)?;
        if !matches!(field(branch,"name")?, Value::String(name) if !name.is_empty()) {
            return Err(malformed("default branch name is missing"));
        }
        commit(object(field(branch, "target")?)?, &mut truncated)?;
    }
    for pr in connection(fields, "pullRequests", &mut truncated)? {
        let pr = object(pr)?;
        if !matches!(field(pr,"headRefOid")?, Value::String(value) if !value.is_empty()) {
            return Err(malformed("snapshot PR head is missing"));
        }
        connection(pr, "reviews", &mut truncated)?;
        connection(pr, "files", &mut truncated)?;
        for node in connection(pr, "commits", &mut truncated)? {
            let head = object(field(object(node)?, "commit")?)?;
            commit(head, &mut truncated)?;
            if field(head, "oid")? != field(pr, "headRefOid")? {
                return Err(malformed("snapshot head and check commit disagree"));
            }
        }
    }
    Ok((repo, truncated))
}

pub fn envelope(
    repo: Value,
    source: &str,
    age_ms: Option<u64>,
    stale: bool,
    truncated: bool,
    captured_at_ms: u64,
    reason: &str,
) -> io::Result<Vec<u8>> {
    let freshness = BTreeMap::from([
        ("source".into(), Value::String(source.into())),
        ("stale".into(), Value::Bool(stale)),
        ("age_ms".into(), age_ms.map_or(Value::Null, Value::Unsigned)),
        ("captured_at_ms".into(), Value::Unsigned(captured_at_ms)),
        ("reason".into(), Value::String(reason.into())),
    ]);
    let doc = BTreeMap::from([
        ("schema".into(), Value::Unsigned(1)),
        ("repository".into(), repo),
        ("freshness".into(), Value::Object(freshness)),
        ("truncated".into(), Value::Bool(truncated)),
    ]);
    let mut bytes = json::encode(&Value::Object(doc), Layout::Compact).map_err(io::Error::other)?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_stays_read_only_compact_and_explicitly_revalidating() {
        let spec = parse(
            &["-R".into(), "github.com/O/R".into(), "--fresh".into()],
            None,
            "github.com",
        )
        .expect("spec");
        let args = spec.args();
        let query = args
            .iter()
            .find_map(|arg| arg.strip_prefix("query="))
            .expect("query");
        assert_eq!(
            crate::operation::classify(query, None).expect("query parses"),
            crate::operation::Operation::Read
        );
        assert!(
            !query.contains('\n'),
            "batch wrapper arguments remain a single line"
        );
        assert!(args.iter().any(|arg| arg == "Cache-Control: no-cache"));
        assert_eq!(spec.owner, "o");
        assert_eq!(spec.repo, "r");
        assert!(
            parse(
                &[
                    "-R".into(),
                    "o/r".into(),
                    "--fresh".into(),
                    "--allow-stale".into()
                ],
                None,
                "github.com"
            )
            .is_err()
        );
    }
    #[test]
    fn repository_binding_and_limits_fail_at_the_boundary() {
        for limit in ["0", "101", "invalid"] {
            assert!(
                parse(
                    &["-R".into(), "o/r".into(), "--limit".into(), limit.into()],
                    None,
                    "github.com"
                )
                .is_err()
            );
        }
        assert!(
            parse(
                &[
                    "-R".into(),
                    "enterprise.example/o/r".into(),
                    "--hostname".into(),
                    "github.com".into()
                ],
                None,
                "github.com"
            )
            .is_err()
        );
        assert!(parse(&[], None, "github.com").is_err());
    }
}
