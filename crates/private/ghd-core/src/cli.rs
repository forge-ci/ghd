//! The gh-compatible command surface. Everything here runs in the caller's
//! process and talks to the daemon through [`crate::client`].

use crate::proto::{PrViewRequest, PrViewResponse};
use crate::{client, version_banner};
use prost::Message;
use std::io::{self, IsTerminal, Write};

pub const USAGE: &str = "\
ghd: a cached, batched, budget-aware gateway for gh reads

usage:
  ghd pr view <number> [-R owner/repo] --json f1,f2   like `gh pr view --json`
  ghd api graphql -f query=... [-f k=v] [-F k=v]      like `gh api graphql`
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
        ["pr", "view", rest @ ..] => pr_view(rest),
        ["api", "graphql", rest @ ..] => crate::graphql::run(rest),
        _ => Err(invalid(USAGE)),
    }
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
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match *arg {
            "-R" | "--repo" => repo = it.next().map(|s| s.to_string()),
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
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| invalid("-R must be owner/repo"))?;
    Ok(PrViewRequest {
        owner: owner.into(),
        repo: name.into(),
        number,
        fields: fields
            .split(',')
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .map(String::from)
            .collect(),
        compact: false,
    })
}

fn pr_view(args: &[&str]) -> io::Result<()> {
    let mut request = parse_pr_view(args, repo_from_git)?;
    request.compact = !io::stdout().is_terminal();
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
    let (status, body) = client::call("POST", "/v1/pr/view", &request.encode_to_vec())?;
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
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()?;
    owner_repo_from_url(String::from_utf8_lossy(&output.stdout).trim())
        .ok_or_else(|| io::Error::other("no github origin remote; pass -R owner/repo"))
}

pub fn owner_repo_from_url(url: &str) -> Option<String> {
    let path = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    path.contains('/').then(|| path.to_string())
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
        // The CLI decides `compact` from its stdout; parsing never sets it.
        assert!(!req.compact);
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
