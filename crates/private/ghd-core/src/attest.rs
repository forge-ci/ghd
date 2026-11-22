//! Local-only transport of soldr evidence into the current commit message.
use kernal_api::hash::{Sha256Digest, sha256_bytes};
use kernal_api::json::{self, Layout, Value};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

pub(crate) fn run(args: &[&str]) -> io::Result<()> {
    let mut path = None;
    let mut dry_run = false;
    let mut lane = "lint";
    let mut rest = args;
    while !rest.is_empty() {
        rest = match rest {
            ["--evidence", value, tail @ ..] if path.is_none() => {
                path = Some(*value);
                tail
            }
            ["--lane", value, tail @ ..] if !value.is_empty() => {
                lane = value;
                tail
            }
            ["--dry-run", tail @ ..] if !dry_run => {
                dry_run = true;
                tail
            }
            _ => {
                return Err(invalid(
                    "usage: ghd attest stamp --evidence <path|-> [--dry-run] [--lane <lane>]",
                ));
            }
        };
    }
    let path = path.ok_or_else(|| invalid("--evidence is required"))?;
    let mut input = Vec::new();
    let reader: Box<dyn Read> = if path == "-" {
        Box::new(io::stdin().lock())
    } else {
        Box::new(std::fs::File::open(path)?)
    };
    reader
        .take(json::MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut input)?;
    let trailers = stamp(Path::new("."), &input, lane, dry_run)?;
    writeln!(io::stdout().lock(), "{trailers}")
}

fn git(repo: &Path, args: &[&str], input: Option<&[u8]>) -> io::Result<String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(input) = input {
        child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("git stdin unavailable"))?
            .write_all(input)?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout).map_err(io::Error::other)
}

fn object(value: &Value) -> io::Result<&BTreeMap<String, Value>> {
    match value {
        Value::Object(value) => Ok(value),
        _ => Err(invalid("evidence must contain JSON objects")),
    }
}

fn string<'a>(value: &'a BTreeMap<String, Value>, key: &str) -> io::Result<&'a str> {
    match value.get(key) {
        Some(Value::String(value)) if !value.is_empty() => Ok(value),
        _ => Err(invalid(&format!("evidence requires string {key}"))),
    }
}

// Python json.dumps(..., sort_keys=True, separators=(",", ":")) defaults to
// ensure_ascii=True. Escape non-ASCII UTF-16 units, including surrogate pairs.
fn canonical(value: &Value) -> io::Result<String> {
    let encoded =
        String::from_utf8(json::encode(value, Layout::Compact).map_err(io::Error::other)?)
            .map_err(io::Error::other)?;
    let mut output = String::new();
    for ch in encoded.chars() {
        if ch.is_ascii() && ch != '\x7f' {
            output.push(ch);
        } else {
            for unit in ch.encode_utf16(&mut [0; 2]) {
                use std::fmt::Write;
                write!(output, "\\u{unit:04x}").map_err(io::Error::other)?;
            }
        }
    }
    Ok(output)
}

fn stamp(repo: &Path, input: &[u8], lane: &str, dry_run: bool) -> io::Result<String> {
    let evidence = json::parse(input).map_err(io::Error::other)?;
    let evidence = object(&evidence)?;
    if evidence.get("v") != Some(&Value::Signed(1)) {
        return Err(invalid("unsupported evidence schema: expected v:1"));
    }
    if !git(
        repo,
        &["status", "--porcelain", "--untracked-files=all"],
        None,
    )?
    .is_empty()
    {
        return Err(invalid("cannot attest: working tree is dirty"));
    }
    let head = git(repo, &["rev-parse", "HEAD"], None)?;
    let tree = git(repo, &["rev-parse", "HEAD^{tree}"], None)?;
    let tree = tree.trim();
    if string(evidence, "tree")? != tree {
        return Err(invalid(
            "cannot attest: evidence tree does not match HEAD tree",
        ));
    }
    let parents = git(repo, &["log", "-1", "--format=%P"], None)?;
    let parents: Vec<_> = parents.split_whitespace().collect();
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    let host = string(evidence, "host")?;
    let Some(Value::Array(results)) = evidence.get("results") else {
        return Err(invalid("evidence requires results array"));
    };
    let mut trailers = Vec::new();
    for result in results {
        let result = object(result)?;
        match result.get("ok") {
            Some(Value::Bool(false)) => continue,
            Some(Value::Bool(true)) => {}
            _ => return Err(invalid("evidence result requires boolean ok")),
        }
        let secs = result
            .get("secs")
            .filter(|value| matches!(value, Value::Signed(0..) | Value::Unsigned(_)))
            .ok_or_else(|| invalid("evidence result requires nonnegative integer secs"))?;
        let mut body: BTreeMap<String, Value> = [
            ("v", Value::Signed(1)),
            ("gate", Value::String(string(result, "id")?.into())),
            ("tree", Value::String(tree.into())),
            (
                "parents",
                Value::Array(parents.iter().map(|p| Value::String((*p).into())).collect()),
            ),
            ("lane", Value::String(lane.into())),
            ("key", Value::String(string(result, "key")?.into())),
            ("via", Value::String("soldr".into())),
            ("secs", secs.clone()),
            ("host", Value::String(host.into())),
            ("at", Value::Unsigned(at)),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value))
        .collect();
        let payload = format!(
            "{tree}\n{}\n{}",
            parents.join(","),
            canonical(&Value::Object(body.clone()))?
        );
        let digest: Sha256Digest = sha256_bytes(payload.as_bytes());
        body.insert("stamp".into(), Value::String(digest.to_hex()[..32].into()));
        trailers.push(format!(
            "Ci-Attestation: {}",
            canonical(&Value::Object(body))?
        ));
    }
    if trailers.is_empty() {
        return Err(invalid("cannot attest: no successful results"));
    }
    let trailers_text = trailers.join("\n");
    if !dry_run {
        amend(repo, &head, &trailers)?;
    }
    Ok(trailers_text)
}

fn amend(repo: &Path, head: &str, trailers: &[String]) -> io::Result<()> {
    let message = git(repo, &["log", "-1", "--format=%B"], None)?;
    let mut args = vec![
        "-c",
        "trailer.Ci-Attestation.key=Ci-Attestation",
        "interpret-trailers",
        "--no-divider",
        "--where=end",
        "--if-exists=add",
        "--if-missing=add",
    ];
    for trailer in trailers {
        args.extend(["--trailer", trailer]);
    }
    let amended = git(repo, &args, Some(message.as_bytes()))?;
    // Abort if HEAD or the worktree changed while preparing the message.
    if git(repo, &["rev-parse", "HEAD"], None)? != head
        || !git(
            repo,
            &["status", "--porcelain", "--untracked-files=all"],
            None,
        )?
        .is_empty()
    {
        return Err(invalid("cannot attest: HEAD or working tree changed"));
    }
    git(
        repo,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--amend",
            "--allow-empty",
            "--only",
            "--no-gpg-sign",
            "--cleanup=verbatim",
            "-F",
            "-",
        ],
        Some(amended.as_bytes()),
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "attest_tests.rs"]
mod tests;
