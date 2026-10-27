//! Safe read shapes: bind targets, share JSON resources, retain native fallbacks.

use crate::{compat, fields, identity};
use kernal_api::json::{self, Layout, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io;

const ISSUE_BASE: &[&str] = &[
    "number",
    "id",
    "title",
    "url",
    "body",
    "state",
    "createdAt",
    "updatedAt",
    "closedAt",
];
const RUN_BASE: &[&str] = &[
    "databaseId",
    "number",
    "displayTitle",
    "headBranch",
    "headSha",
    "event",
    "status",
    "conclusion",
    "createdAt",
    "startedAt",
    "updatedAt",
    "url",
    "workflowDatabaseId",
    "attempt",
    "name",
];
const CHECK_FIELDS: &[&str] = &[
    "bucket",
    "completedAt",
    "description",
    "event",
    "link",
    "name",
    "startedAt",
    "state",
    "workflow",
];
const SHARED_LIMIT: usize = 100;

#[derive(Clone, Debug)]
pub struct ReadPlan {
    pub host: String,
    pub repo: Option<String>,
    pub kind: String,
    pub action: String,
    pub fetch_args: Vec<String>,
    pub key_args: Vec<String>,
    pub fields: Vec<String>,
    pub fetch_fields: Vec<String>,
    pub limit: Option<usize>,
    pub array: bool,
    pub resource: bool,
    pub jq: Option<String>,
    pub template: Option<String>,
    pub filters: BTreeMap<String, String>,
    pub worktree_dependent: bool,
    pub rest: Option<crate::rest::RestPlan>,
}

#[derive(Default)]
struct Arguments {
    values: BTreeMap<String, String>,
    flags: BTreeSet<String>,
    positional: Vec<String>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn flag<'a>(kind: &str, action: &str, value: &'a str) -> &'a str {
    match value {
        "-R" => "--repo",
        "-q" => "--jq",
        "-t" => "--template",
        "-L" => "--limit",
        "-S" => "--search",
        "-l" => "--label",
        "-H" => "--head",
        "-B" => "--base",
        "-b" if kind == "run" => "--branch",
        "-c" if kind == "run" => "--commit",
        "-e" if kind == "run" => "--event",
        "-u" if kind == "run" => "--user",
        "-w" if kind == "run" => "--workflow",
        "-a" if kind == "run" && action == "view" => "--attempt",
        "-a" if kind == "run" && action == "list" => "--all",
        "-a" if matches!(kind, "pr" | "issue") => "--author",
        "-s" if kind == "run" => "--status",
        "-s" if matches!(kind, "pr" | "issue") => "--state",
        _ => value,
    }
}

fn parse(kind: &str, action: &str, args: &[String]) -> io::Result<Option<Arguments>> {
    let mut out = Arguments::default();
    let mut it = args.iter();
    while let Some(word) = it.next() {
        let (name, attached) = word
            .split_once('=')
            .map_or((word.as_str(), None), |(n, v)| (n, Some(v)));
        let name = flag(kind, action, name);
        match name {
            "--repo" | "--json" | "--jq" | "--template" | "--limit" | "--attempt" | "--branch"
            | "--commit" | "--created" | "--event" | "--status" | "--user" | "--workflow"
            | "--author" | "--assignee" | "--label" | "--state" | "--search" | "--head"
            | "--base" | "--owner" | "--org" | "--env" | "--ref" | "--language" | "--topic"
            | "--sort" | "--order" | "--license" | "--match" | "--filename" | "--extension"
            | "--visibility" | "--key" => {
                let value = attached
                    .or_else(|| it.next().map(String::as_str))
                    .ok_or_else(|| invalid("read flag is missing its value"))?;
                if out.values.insert(name.into(), value.into()).is_some() {
                    return Ok(None);
                }
            }
            "--all"
            | "--yaml"
            | "--raw"
            | "--log"
            | "--log-failed"
            | "--draft"
            | "--archived"
            | "--no-archived"
            | "--public"
            | "--private"
            | "--source"
            | "--exclude-drafts"
            | "--exclude-pre-releases"
            | "--name-only"
            | "--patch" => {
                if attached.is_some() || !out.flags.insert(name.into()) {
                    return Ok(None);
                }
            }
            value if value.starts_with('-') => return Ok(None),
            _ => out.positional.push(word.clone()),
        }
    }
    Ok(Some(out))
}

fn repo_scoped(kind: &str, action: &str, parsed: &Arguments) -> bool {
    if parsed.values.contains_key("--org") {
        return false;
    }
    matches!(
        kind,
        "pr" | "issue"
            | "run"
            | "workflow"
            | "release"
            | "label"
            | "cache"
            | "ruleset"
            | "secret"
            | "variable"
    ) || kind == "repo" && action == "view"
}

fn policy(kind: &str, action: &str) -> Option<(&'static [&'static str], &'static [&'static str])> {
    match (kind, action) {
        ("pr", "checks") => Some((CHECK_FIELDS, CHECK_FIELDS)),
        ("pr", "view" | "list") => Some((
            fields::PR_FIELDS,
            &[
                "author",
                "assignees",
                "labels",
                "comments",
                "reviews",
                "reviewRequests",
                "reviewDecision",
                "statusCheckRollup",
                "mergeCommit",
                "mergedBy",
                "baseRefOid",
                "commits",
                "files",
                "milestone",
                "latestReviews",
                "headRepository",
                "headRepositoryOwner",
                "isCrossRepository",
                "maintainerCanModify",
                "autoMergeRequest",
                "closingIssuesReferences",
                "projectCards",
                "projectItems",
                "reactionGroups",
                "potentialMergeCommit",
                "fullDatabaseId",
            ],
        )),
        ("issue", "view" | "list") => Some((
            ISSUE_BASE,
            &[
                "author",
                "labels",
                "assignees",
                "milestone",
                "comments",
                "projectCards",
                "projectItems",
                "reactionGroups",
                "closed",
                "isPinned",
                "stateReason",
            ],
        )),
        ("run", "view" | "list") => Some((RUN_BASE, &["workflowName", "jobs"])),
        _ => None,
    }
}

/// Cache binding must be explicit (`--repo`, repository selector, or GH_REPO).
/// Unbound/unknown shapes retain real gh's context resolution and execution.
pub fn prepare(
    args: &[String],
    host: &str,
    default_repo: Option<&str>,
) -> io::Result<Option<ReadPlan>> {
    if args.first().is_some_and(|word| word == "api") {
        return Ok(crate::rest::prepare(args, host)?.map(api_plan));
    }
    let words: Vec<_> = args.iter().map(String::as_str).collect();
    if compat::classify(&words) != compat::Kind::Read || args.len() < 2 {
        return Ok(None);
    }
    let (kind, action) = (args[0].as_str(), args[1].as_str());
    if kind == "run" && action == "watch" {
        return Ok(None);
    }
    let Some(mut parsed) = parse(kind, action, &args[2..])? else {
        return Ok(None);
    };
    if parsed.values.contains_key("--jq") && parsed.values.contains_key("--template") {
        return Ok(None);
    }
    if kind == "repo"
        && action == "view"
        && (parsed.positional.len() > 1 || parsed.values.contains_key("--repo"))
    {
        return Ok(None);
    }
    let scoped = repo_scoped(kind, action, &parsed);
    if parsed.positional.iter().any(|word| word.contains("://")) {
        return Ok(None);
    }
    let selector = if kind == "repo" && action == "view" {
        parsed.positional.first().map(String::as_str)
    } else {
        None
    };
    let binding = parsed
        .values
        .get("--repo")
        .map(String::as_str)
        .or(selector)
        .or(default_repo);
    let (host, repo) = if scoped {
        let Some(binding) = binding else {
            return Ok(None);
        };
        let (bound_host, owner, repo) = identity::repository(binding)?;
        (
            bound_host.unwrap_or(identity::hostname(host)?),
            Some(format!("{owner}/{repo}")),
        )
    } else {
        (identity::hostname(host)?, None)
    };
    let mut plan = ReadPlan {
        host,
        repo,
        kind: kind.into(),
        action: action.into(),
        fetch_args: args.to_vec(),
        key_args: args.to_vec(),
        fields: Vec::new(),
        fetch_fields: Vec::new(),
        limit: None,
        array: false,
        resource: false,
        jq: None,
        template: None,
        filters: BTreeMap::new(),
        worktree_dependent: scoped,
        rest: None,
    };
    if let Some(repo) = &plan.repo {
        // The executable receives a source-bound target, not an inferred fork.
        parsed.values.remove("--repo");
        plan.fetch_args = vec![kind.into(), action.into()];
        if kind == "repo" && action == "view" {
            plan.fetch_args.push(format!("{}/{repo}", plan.host));
        } else {
            plan.fetch_args.extend(parsed.positional.iter().cloned());
            plan.fetch_args
                .extend(["--repo".into(), format!("{}/{repo}", plan.host)]);
        }
        append_options(&mut plan.fetch_args, &parsed);
        plan.key_args.clone_from(&plan.fetch_args);
    }
    prepare_projection(&mut plan, &parsed)?;
    Ok(Some(plan))
}

fn api_plan(rest: crate::rest::RestPlan) -> ReadPlan {
    ReadPlan {
        host: rest.host.clone(),
        repo: None,
        kind: "api".into(),
        action: "GET".into(),
        key_args: rest.fetch_args.clone(),
        fetch_args: rest.fetch_args.clone(),
        fields: Vec::new(),
        fetch_fields: Vec::new(),
        limit: None,
        array: false,
        resource: false,
        jq: rest.jq.clone(),
        template: rest.template.clone(),
        filters: BTreeMap::new(),
        worktree_dependent: false,
        rest: Some(rest),
    }
}

fn append_options(args: &mut Vec<String>, parsed: &Arguments) {
    for (flag, value) in &parsed.values {
        args.extend([flag.clone(), value.clone()]);
    }
    args.extend(parsed.flags.iter().cloned());
}

fn prepare_projection(plan: &mut ReadPlan, parsed: &Arguments) -> io::Result<()> {
    let Some(csv) = parsed.values.get("--json") else {
        return Ok(());
    };
    let Some((base, extra)) = policy(&plan.kind, &plan.action) else {
        return Ok(());
    };
    let wanted: BTreeSet<_> = csv.split(',').map(String::from).collect();
    if wanted
        .iter()
        .any(|field| !base.contains(&field.as_str()) && !extra.contains(&field.as_str()))
    {
        return Ok(());
    }
    if plan.kind == "run" && plan.action == "list" && wanted.contains("jobs") {
        return Ok(());
    }
    let list = plan.action == "list";
    let checks = plan.action == "checks";
    if list && !parsed.positional.is_empty() {
        return Ok(());
    }
    if !list
        && (parsed.positional.len() != 1
            || parsed.positional[0]
                .trim_start_matches('#')
                .parse::<u64>()
                .ok()
                .is_none_or(|id| id == 0))
    {
        return Ok(());
    }
    let default_limit = if plan.kind == "run" { 20 } else { 30 };
    let limit = parsed
        .values
        .get("--limit")
        .map(|v| {
            v.parse::<usize>()
                .map_err(|_| invalid("invalid read limit"))
        })
        .transpose()?
        .unwrap_or(default_limit);
    if list && (limit == 0 || limit > SHARED_LIMIT) {
        return Ok(());
    }
    if !list && parsed.values.contains_key("--limit") {
        return Ok(());
    }
    if let Some(attempt) = parsed.values.get("--attempt")
        && (plan.kind != "run"
            || plan.action != "view"
            || attempt.parse::<u64>().ok().is_none_or(|n| n == 0))
    {
        return Ok(());
    }
    let skip = ["--json", "--jq", "--template", "--limit"];
    let filters: BTreeMap<_, _> = parsed
        .values
        .iter()
        .filter(|(key, _)| !skip.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if !list && filters.keys().any(|key| key != "--attempt") {
        return Ok(());
    }
    let Some(repo) = &plan.repo else {
        return Ok(());
    };
    let mut fetch = vec![plan.kind.clone(), plan.action.clone()];
    if !list {
        fetch.push(
            parsed.positional[0]
                .trim_start_matches('#')
                .parse::<u64>()
                .map_err(|_| invalid("invalid resource number"))?
                .to_string(),
        );
    }
    fetch.extend(["--repo".into(), format!("{}/{repo}", plan.host)]);
    for (key, value) in &filters {
        fetch.extend([key.clone(), value.clone()]);
    }
    fetch.extend(parsed.flags.iter().cloned());
    if list {
        fetch.extend(["--limit".into(), SHARED_LIMIT.to_string()]);
    }
    plan.key_args.clone_from(&fetch);
    set_fields(plan, base, wanted);
    fetch.extend(["--json".into(), plan.fetch_fields.join(",")]);
    plan.fetch_args = fetch;
    plan.limit = list.then_some(limit);
    plan.array = list || checks;
    plan.resource = true;
    plan.worktree_dependent = false;
    plan.jq = parsed.values.get("--jq").cloned();
    plan.template = parsed.values.get("--template").cloned();
    plan.filters = filters;
    Ok(())
}

fn set_fields(plan: &mut ReadPlan, base: &[&str], wanted: BTreeSet<String>) {
    plan.fields = wanted.iter().cloned().collect();
    plan.fetch_fields = base
        .iter()
        .map(|v| (*v).to_string())
        .chain(wanted)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
}

impl ReadPlan {
    pub fn with_fields(&self, additional: &[String]) -> Vec<String> {
        let fields: BTreeSet<_> = self
            .fetch_fields
            .iter()
            .chain(additional)
            .cloned()
            .collect();
        let mut args = self.fetch_args.clone();
        if let Some(index) = args.iter().position(|word| word == "--json") {
            args[index + 1] = fields.into_iter().collect::<Vec<_>>().join(",");
        }
        args
    }

    pub fn project(&self, body: &[u8]) -> io::Result<Vec<u8>> {
        let value =
            json::parse_members(body).map_err(|_| invalid("gh resource response is not JSON"))?;
        let project = |value: &Value| -> io::Result<Value> {
            let Value::ObjectMembers(object) = value else {
                return Err(invalid("gh resource row is not an object"));
            };
            let mut out = BTreeMap::new();
            for field in &self.fields {
                out.insert(
                    field.clone(),
                    object
                        .iter()
                        .rev()
                        .find(|(key, _)| key == field)
                        .map(|(_, value)| value)
                        .ok_or_else(|| {
                            invalid("gh resource response is missing a requested field")
                        })?
                        .clone(),
                );
            }
            Ok(Value::Object(out))
        };
        let value = if self.array {
            let Value::Array(rows) = &value else {
                return Err(invalid("gh resource collection is not an array"));
            };
            Value::Array(
                rows.iter()
                    .take(self.limit.unwrap_or(usize::MAX))
                    .map(project)
                    .collect::<io::Result<_>>()?,
            )
        } else {
            project(&value)?
        };
        let encoded = json::encode(&value, Layout::Compact).map_err(io::Error::other)?;
        // Match Go's JSON exporter escaping, including safe HTML and line separators.
        let text = String::from_utf8(encoded)
            .map_err(io::Error::other)?
            .replace('&', "\\u0026")
            .replace('<', "\\u003c")
            .replace('>', "\\u003e")
            .replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029");
        Ok(format!("{text}\n").into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan(words: &[&str]) -> ReadPlan {
        prepare(
            &words.iter().map(|s| (*s).into()).collect::<Vec<_>>(),
            "github.com",
            None,
        )
        .expect("prepare")
        .expect("cacheable")
    }

    #[test]
    fn resource_field_order_limits_and_flag_order_do_not_partition_shared_data() {
        let a = plan(&[
            "issue",
            "list",
            "-R",
            "o/r",
            "--json",
            "title,state",
            "--limit",
            "2",
            "--label",
            "ready",
        ]);
        let b = plan(&[
            "issue",
            "list",
            "--label",
            "ready",
            "--limit",
            "10",
            "--json",
            "state,title,number",
            "--repo",
            "github.com/o/r",
        ]);
        assert!(a.resource && b.resource);
        assert_eq!(a.key_args, b.key_args);
        assert_eq!(a.limit, Some(2));
        assert_eq!(b.limit, Some(10));
        let other = plan(&[
            "issue", "list", "-R", "o/r", "--json", "title", "--label", "other",
        ]);
        assert_ne!(a.key_args, other.key_args);
    }

    #[test]
    fn projection_preserves_nested_nulls_and_escapes_like_gh() {
        let p = plan(&[
            "pr",
            "view",
            "42",
            "-R",
            "o/r",
            "--json",
            "reviews,author,title",
        ]);
        let body =
            br#"{"number":42,"author":null,"reviews":[{"state":"APPROVED"}],"title":"<hello>&"}"#;
        assert_eq!(
            p.project(body).expect("projection"),
            br#"{"author":null,"reviews":[{"state":"APPROVED"}],"title":"\u003chello\u003e\u0026"}
"#
        );
        assert!(p.project(br#"{"title":"missing fields"}"#).is_err());
    }

    #[test]
    fn url_selectors_remain_native_before_identity_pinned_execution() {
        for kind in ["pr", "issue"] {
            for repo in ["o/r", "github.com/o/r"] {
                let args = [
                    kind,
                    "view",
                    "https://other.example/other/repo/pull/42",
                    "--repo",
                    repo,
                    "--json",
                    "title",
                ];
                assert!(
                    prepare(&args.map(String::from), "github.com", None)
                        .expect("parse")
                        .is_none()
                );
            }
            let args = [
                kind,
                "view",
                "https://other.example/other/repo/issues/42",
                "--json",
                "title",
            ];
            assert!(
                prepare(&args.map(String::from), "github.com", Some("o/r"))
                    .expect("default repo")
                    .is_none()
            );
        }
        let args = ["run", "watch", "42", "--repo", "o/r"];
        assert!(
            prepare(&args.map(String::from), "github.com", None)
                .expect("watch")
                .is_none()
        );
    }

    #[test]
    fn attempts_and_unsupported_or_large_shapes_remain_distinct() {
        let a = plan(&[
            "run",
            "view",
            "42",
            "-R",
            "o/r",
            "--json",
            "status",
            "--attempt",
            "1",
        ]);
        let b = plan(&[
            "run",
            "view",
            "42",
            "-R",
            "o/r",
            "--json",
            "status",
            "--attempt",
            "2",
        ]);
        assert_ne!(a.key_args, b.key_args);
        let large = plan(&[
            "run", "list", "-R", "o/r", "--json", "status", "--limit", "101",
        ]);
        assert!(!large.resource, "native pagination remains authoritative");
        let unsupported = plan(&["issue", "view", "42", "-R", "o/r", "--json", "futureField"]);
        assert!(
            !unsupported.resource,
            "unknown schema keeps exact native output"
        );
        assert!(
            prepare(&["issue".into(), "list".into()], "github.com", None)
                .expect("unbound")
                .is_none()
        );
    }
}
