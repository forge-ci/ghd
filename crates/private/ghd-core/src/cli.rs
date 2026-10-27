//! The gh-compatible command surface. Everything here runs in the caller's
//! process and talks to the daemon through [`crate::client`].

use crate::proto::{PrViewRequest, PrViewResponse};
use crate::{client, version_banner};
use prost::Message;
use std::io::{self, Write};

pub const USAGE: &str = "\
ghd: a cached, batched, budget-aware gateway for gh reads

usage:
  ghd gh <args...>                                forward to real gh safely
  ghd pr view <number> [-R owner/repo] --json f1,f2   like `gh pr view --json`
  ghd api graphql [--hostname host] -f query=...     like `gh api graphql`
  ghd stats                                           what the daemon saved
  ghd daemon                                          run the daemon (auto-started)
  ghd --version | --help";

/// Run every subcommand except `daemon`, which needs `ghd-daemon`.
pub fn run(args: &[String]) -> io::Result<()> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] | ["--help" | "-h" | "help"] => print(USAGE),
        ["--version" | "-V"] => print(&version_banner()),
        ["stats"] => stats(),
        ["gh", rest @ ..] => crate::reads::run_or_passthrough(rest),
        ["pr", "view", rest @ ..] if native_pr_args(rest) => pr_view(rest),
        ["api", "graphql", rest @ ..] if native_graphql_args(rest) => crate::graphql::run(rest),
        _ => crate::reads::run_or_passthrough(&args),
    }
}

fn native_pr_args(args: &[&str]) -> bool {
    let mut it = args.iter();
    let mut json = false;
    while let Some(arg) = it.next() {
        match *arg {
            "--json" => {
                json = true;
                it.next();
            }
            "-R" | "--repo" | "--hostname" => {
                it.next();
            }
            value if !value.starts_with('-') => {}
            _ => return false,
        }
    }
    json
}

fn native_graphql_args(args: &[&str]) -> bool {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match *arg {
            "-f" | "--raw-field" | "-F" | "--field" | "--hostname" => {
                it.next();
            }
            value if (value.starts_with("-f") || value.starts_with("-F")) && value.len() > 2 => {}
            _ => return false,
        }
    }
    true
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_string())
}

fn print(text: &str) -> io::Result<()> {
    writeln!(io::stdout().lock(), "{text}")
}

fn stats() -> io::Result<()> {
    let (status, body) = client::call("GET", "/v1/stats", &[])?;
    if status != 200 {
        return Err(io::Error::other(format!("daemon HTTP {status}")));
    }
    print(&String::from_utf8_lossy(&body))
}

/// Parse `pr view` arguments into a request, without touching the network.
pub fn parse_pr_view(
    args: &[&str],
    default_repo: impl FnOnce() -> io::Result<String>,
) -> io::Result<PrViewRequest> {
    let mut number = None;
    let mut repo = None;
    let mut fields = None;
    let mut host = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match *arg {
            "-R" | "--repo" => {
                let value = it.next().ok_or_else(|| invalid("--repo needs a value"))?;
                if repo.replace(value.to_string()).is_some() {
                    return Err(invalid("repeated repository selector"));
                }
            }
            "--hostname" => {
                let value = it
                    .next()
                    .ok_or_else(|| invalid("--hostname needs a value"))?;
                if host.replace(crate::identity::hostname(value)?).is_some() {
                    return Err(invalid("repeated hostname selector"));
                }
            }
            "--json" => fields = it.next().map(|s| s.to_string()),
            other => {
                number = Some(
                    other
                        .trim_start_matches('#')
                        .parse::<u64>()
                        .map_err(|_| invalid("bad PR number"))?,
                );
            }
        }
    }
    let number = number.ok_or_else(|| invalid("missing PR number"))?;
    let fields = fields.ok_or_else(|| invalid("--json f1,f2 is required"))?;
    let repo = match repo {
        Some(r) => r,
        None => default_repo()?,
    };
    let (hostname, owner, name) = crate::identity::bound_repository(&repo, host.as_deref())?;
    let hostname = hostname.unwrap_or_default();
    Ok(PrViewRequest {
        owner,
        repo: name,
        number,
        hostname,
        fields: fields
            .split(',')
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .map(String::from)
            .collect(),
    })
}

fn pr_view(args: &[&str]) -> io::Result<()> {
    let request = parse_pr_view(args, repo_from_git)?;
    let reply = pr_view_request(&request)?;
    if std::env::var_os("GHD_DEBUG").is_some() {
        // One write per line, so concurrent ghd processes sharing a pipe
        // never interleave mid-line.
        io::stderr().write_all(format!("ghd: {}\n", reply.source).as_bytes())?;
    }
    print(&String::from_utf8_lossy(&reply.json))
}

/// One `pr view` round trip; the Python binding calls this directly.
pub fn pr_view_request(request: &PrViewRequest) -> io::Result<PrViewResponse> {
    if request.fields.is_empty() {
        return Err(invalid("at least one JSON field is required"));
    }
    if request
        .fields
        .iter()
        .any(|field| !crate::fields::PR_FIELDS.contains(&field.as_str()))
    {
        let repo = if request.hostname.is_empty() {
            format!("{}/{}", request.owner, request.repo)
        } else {
            format!("{}/{}/{}", request.hostname, request.owner, request.repo)
        };
        let args = vec![
            "pr".into(),
            "view".into(),
            request.number.to_string(),
            "--repo".into(),
            repo,
            "--json".into(),
            request.fields.join(","),
        ];
        let response = crate::reads::read(&args)?;
        if response.exit_code != 0 {
            return Err(io::Error::other(
                String::from_utf8_lossy(&response.stderr).into_owned(),
            ));
        }
        let mut json = response.stdout;
        if json.last() == Some(&b'\n') {
            json.pop();
        }
        return Ok(PrViewResponse {
            json,
            source: response.source,
            error: String::new(),
        });
    }
    let (status, body) = client::call_host(
        (!request.hostname.is_empty()).then_some(request.hostname.as_str()),
        "POST",
        "/v1/pr/view",
        &request.encode_to_vec(),
    )?;
    let reply = PrViewResponse::decode(body.as_slice())
        .map_err(|_| io::Error::other(format!("daemon HTTP {status}: undecodable reply")))?;
    if reply.error.is_empty() {
        Ok(reply)
    } else {
        Err(io::Error::other(reply.error))
    }
}

/// `owner/repo` from the `origin` remote, as gh infers it.
fn repo_from_git() -> io::Result<String> {
    if let Ok(repo) = std::env::var("GH_REPO") {
        return Ok(repo);
    }
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("no origin remote; pass -R owner/repo"));
    }
    let remote = String::from_utf8_lossy(&output.stdout);
    let (host, owner, name) = crate::identity::repository(remote.trim())?;
    Ok(host.map_or_else(
        || format!("{owner}/{name}"),
        |host| format!("{host}/{owner}/{name}"),
    ))
}

/// Legacy GitHub.com-only helper; general host-aware resolution uses
/// `identity::repository` so this compatibility API retains its old scope.
pub fn owner_repo_from_url(url: &str) -> Option<String> {
    let (host, owner, repo) = crate::identity::repository(url).ok()?;
    (host.as_deref() == Some("github.com")).then(|| format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_git() -> io::Result<String> {
        Err(io::Error::other("unused"))
    }

    #[test]
    fn parses_gh_style_pr_view_arguments() {
        let req = parse_pr_view(&["#12", "-R", "o/r", "--json", "state, title"], no_git)
            .expect("test fixture");
        assert_eq!(
            (req.owner.as_str(), req.repo.as_str(), req.number),
            ("o", "r", 12)
        );
        assert_eq!(req.fields, ["state", "title"]);
    }

    #[test]
    fn repository_host_and_explicit_host_must_agree() {
        let req = parse_pr_view(
            &["12", "-R", "GIT.EXAMPLE.COM/o/r", "--json", "state"],
            no_git,
        )
        .expect("parses");
        assert_eq!(req.hostname, "git.example.com");
        assert!(
            parse_pr_view(
                &[
                    "12",
                    "-R",
                    "git.example.com/o/r",
                    "--hostname",
                    "other.example.com",
                    "--json",
                    "state"
                ],
                no_git
            )
            .is_err()
        );
        assert!(
            parse_pr_view(&["12", "-R", "o/r", "-R", "x/y", "--json", "state"], no_git).is_err()
        );
        let default = parse_pr_view(&["12", "--json", "state"], || {
            Ok("git.example.com/o/r".into())
        })
        .expect("default repo");
        assert_eq!(req, default);
    }

    #[test]
    fn requires_a_number_and_fields() {
        assert!(parse_pr_view(&["-R", "o/r", "--json", "state"], no_git).is_err());
        assert!(parse_pr_view(&["1", "-R", "o/r"], no_git).is_err());
    }

    #[test]
    fn reads_owner_repo_from_every_github_remote_form() {
        for url in [
            "git@github.com:o/r.git",
            "https://github.com/o/r",
            "ssh://git@github.com/o/r.git",
        ] {
            assert_eq!(owner_repo_from_url(url).as_deref(), Some("o/r"), "{url}");
        }
        assert_eq!(owner_repo_from_url("https://gitlab.com/o/r"), None);
    }
}
