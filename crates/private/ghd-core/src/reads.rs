//! Client-side read routing shared by CLI and native Python bindings.

use crate::proto::{EnvironmentVariable, GhReadRequest, GhReadResponse};
use crate::{client, compat, resource};
use prost::Message;
use std::io::{self, IsTerminal, Write};
use std::time::UNIX_EPOCH;

/// Only output/configuration context crosses the wire; credentials are resolved
/// independently and the daemon pins its own identity's token in its children.
pub const CONTEXT_ENV: &[&str] = &[
    "GH_CONFIG_DIR",
    "GH_REPO",
    "GH_FORCE_TTY",
    "GH_COLOR_LABELS",
    "GH_ACCESSIBLE_COLORS",
    "GH_PAGER",
    "GH_PROMPT_DISABLED",
    "GH_MDV2",
    "NO_COLOR",
    "CLICOLOR",
    "CLICOLOR_FORCE",
    "PAGER",
    "PATH",
    "PATHEXT",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_TIME",
    "TZ",
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "CURL_CA_BUNDLE",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "all_proxy",
];

fn text_path(path: &std::path::Path) -> io::Result<String> {
    path.to_str()
        .map(String::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "gh read paths must be UTF-8"))
}

fn worktree_stamp() -> io::Result<String> {
    let spec = kernal_api::SpawnSpec::new("git")
        .args([
            "show",
            "--no-patch",
            "--no-show-signature",
            "--format=%H:%D",
            "HEAD",
        ])
        .stdin(kernal_api::StreamMode::Null);
    let result =
        kernal_api::run_bounded_command(spec, std::time::Duration::from_secs(5), 64 * 1024)
            .map_err(io::Error::other)?;
    if !result.exit.is_success() {
        return Ok("no-git-head".into());
    }
    Ok(kernal_api::hash::blake3_bytes(&result.stdout).to_hex())
}

pub fn uncached_context() -> bool {
    ["GH_FORCE_TTY", "CLICOLOR_FORCE", "GH_DEBUG", "DEBUG"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

pub fn prepare(args: &[String]) -> io::Result<Option<GhReadRequest>> {
    if uncached_context() {
        return Ok(None);
    }
    let host = std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".into());
    let repo = std::env::var("GH_REPO")
        .ok()
        .filter(|repo| !repo.is_empty());
    let Some(plan) = resource::prepare(args, &host, repo.as_deref())? else {
        return Ok(None);
    };
    backend_request(
        args,
        plan.host,
        repo.unwrap_or_default(),
        plan.worktree_dependent,
    )
    .map(Some)
}

/// Build the caller-bound executable/environment context for daemon read APIs.
pub fn backend_request(
    args: &[String],
    hostname: String,
    default_repo: String,
    worktree_dependent: bool,
) -> io::Result<GhReadRequest> {
    let command = compat::command()?;
    let cwd = std::env::current_dir()?;
    let path = std::path::Path::new(command.get_program());
    // Keep an ordinary absolute spelling: Windows cmd wrappers cannot consume
    // the verbatim prefix that canonicalize adds. Validation still resolves
    // the target when rejecting recursion.
    let program = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let metadata = program.metadata()?;
    let stamp = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let environment = CONTEXT_ENV
        .iter()
        .map(|name| {
            let mut value = std::env::var(name).unwrap_or_default();
            if matches!(
                *name,
                "GH_CONFIG_DIR"
                    | "XDG_CONFIG_HOME"
                    | "XDG_CACHE_HOME"
                    | "SSL_CERT_FILE"
                    | "SSL_CERT_DIR"
                    | "CURL_CA_BUNDLE"
            ) && !value.is_empty()
            {
                let path = std::path::Path::new(&value);
                if path.is_relative() {
                    value = text_path(&cwd.join(path))?;
                }
            }
            Ok(EnvironmentVariable {
                name: (*name).into(),
                value,
            })
        })
        .collect::<io::Result<_>>()?;
    Ok(GhReadRequest {
        args: args.to_vec(),
        cwd: text_path(&cwd)?,
        program: text_path(&program)?,
        program_stamp: format!("{}:{stamp}", metadata.len()),
        default_repo,
        worktree_stamp: if worktree_dependent {
            worktree_stamp()?
        } else {
            String::new()
        },
        environment,
        hostname,
    })
}

pub fn request_reply(request: &GhReadRequest) -> io::Result<GhReadResponse> {
    // Resolve the requested host before endpoint lookup; client::call_host pins
    // exactly this identity when spawning, as for the direct GraphQL/PR APIs.
    let (status, body) = client::call_host(
        Some(&request.hostname),
        "POST",
        "/v1/read/gh",
        &request.encode_to_vec(),
    )?;
    let response = GhReadResponse::decode(body.as_slice()).map_err(|_| io::Error::other(format!(
        "daemon HTTP {status}: cached gh reads unsupported or reply invalid; restart the daemon after upgrading"
    )))?;
    if response.error.is_empty() {
        Ok(response)
    } else {
        Err(io::Error::other(response.error))
    }
}

/// Read-only binding entry point. Local/interactive/write commands are rejected
/// here; the CLI's compatibility route still executes them with original streams.
pub fn read(args: &[String]) -> io::Result<GhReadResponse> {
    let request = prepare(args)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "read needs a known safe shape, an explicit repository binding where applicable, and no debug/forced-terminal context",
        )
    })?;
    request_reply(&request)
}

pub fn run_or_passthrough(args: &[&str]) -> io::Result<()> {
    if io::stdout().is_terminal() || uncached_context() {
        if crate::request_policy::current().active() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cache controls require a headless managed read",
            ));
        }
        return compat::run(args);
    }
    let words: Vec<_> = args.iter().map(|s| (*s).to_string()).collect();
    let Some(request) = prepare(&words)? else {
        if crate::request_policy::current().active() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cache controls require a known safe read",
            ));
        }
        return compat::run(args);
    };
    let response = request_reply(&request)?;
    io::stdout().lock().write_all(&response.stdout)?;
    io::stderr().lock().write_all(&response.stderr)?;
    if std::env::var_os("GHD_DEBUG").is_some() {
        writeln!(io::stderr().lock(), "ghd: {}", response.source)?;
    }
    if response.exit_code == 0 {
        Ok(())
    } else {
        Err(compat::exit_error(response.exit_code))
    }
}
