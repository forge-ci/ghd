//! Real gh's jq/template engine over an authenticated private loopback body.

use super::server::{State, new_token};
use ghd_core::{
    proto::{GhReadRequest, GhReadResponse},
    resource::ReadPlan,
};
use kernal_api::{
    SpawnSpec, StreamMode, async_engine,
    http_server::{self, Response, Server},
};
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

struct RenderDirectory(PathBuf);
impl Drop for RenderDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub async fn render(
    state: &State,
    request: &GhReadRequest,
    plan: &ReadPlan,
    body: Vec<u8>,
) -> io::Result<GhReadResponse> {
    let nonce = new_token().await?;
    crate::platform::fs::create_private_dir(&state.render_root)?;
    let name =
        kernal_api::hash::blake3_bytes(format!("ghd-render-directory-v1:{nonce}").as_bytes())
            .to_hex();
    let directory = RenderDirectory(state.render_root.join(name));
    std::fs::create_dir(&directory.0)?;
    let server = Server::bind(
        SocketAddr::from(([127, 0, 0, 1], 0)),
        http_server::Limits::default(),
    )
    .await?;
    let port = server.local_addr()?.port();
    let expected_token = format!("token {nonce}");
    let expected_bearer = format!("Bearer {nonce}");
    let body = Arc::new(body);
    let task = async_engine::launch(server.serve(move |request| {
        let body = Arc::clone(&body);
        let authorized = request.header("authorization").is_some_and(|header| {
            header == expected_token.as_bytes() || header == expected_bearer.as_bytes()
        });
        async move {
            let (status, bytes) = if !authorized {
                (401, b"unauthorized".to_vec())
            } else if request.method() != "GET" || request.path() != "/ghd/cache/render" {
                (404, b"local rendering only".to_vec())
            } else {
                (200, body.as_ref().clone())
            };
            Response::new(status, bytes)
                .and_then(|r| r.with_header("content-type", "application/json"))
                .unwrap_or_default()
        }
    }));
    let mut args = vec![
        "api".into(),
        format!("http://127.0.0.1:{port}/ghd/cache/render"),
        "--hostname".into(),
        "127.0.0.1".into(),
    ];
    if let Some(jq) = &plan.jq {
        args.extend(["--jq".into(), jq.clone()]);
    }
    if let Some(template) = &plan.template {
        args.extend(["--template".into(), template.clone()]);
    }
    let spec = render_spec(request, &directory.0, &nonce, args);
    let permit = state.gh_permits.acquire().await;
    // The blocking owner retains the directory until its child is reaped, even
    // if an HTTP caller disconnects while rendering is in progress.
    let result = async_engine::launch_blocking(move || {
        let _directory = directory;
        let _permit = permit;
        kernal_api::run_bounded_command(spec, Duration::from_secs(60), 32 * 1024 * 1024)
    })
    .await
    .map_err(io::Error::other)?
    .map_err(io::Error::other);
    task.cancel();
    let _ = task.await;
    let output = result?;
    let raw = output.exit.raw_code();
    Ok(GhReadResponse {
        stdout: output.stdout,
        stderr: output.stderr,
        exit_code: if raw < 0 { 128 - raw } else { raw },
        source: "rendered-locally".into(),
        ..Default::default()
    })
}

fn render_spec(
    request: &GhReadRequest,
    directory: &std::path::Path,
    nonce: &str,
    args: Vec<String>,
) -> SpawnSpec {
    let mut spec = SpawnSpec::new(&request.program)
        .args(args)
        .clear_env(true)
        .current_dir(directory)
        .stdin(StreamMode::Null)
        .kill_when_owner_dies(true);
    for name in [
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "TMP",
        "TEMP",
        "TMPDIR",
        "LD_LIBRARY_PATH",
        "DYLD_LIBRARY_PATH",
    ] {
        if let Some(value) = std::env::var_os(name) {
            spec = spec.env(name, value);
        }
    }
    for variable in &request.environment {
        if ["LANG", "LC_ALL", "LC_CTYPE", "LC_TIME", "TZ"].contains(&variable.name.as_str()) {
            spec = spec.env(&variable.name, &variable.value);
        }
    }
    spec.env("GH_CONFIG_DIR", directory)
        .env("GH_HOST", "127.0.0.1")
        .env("GH_TOKEN", nonce)
        .env("GITHUB_TOKEN", nonce)
        .env("GH_ENTERPRISE_TOKEN", nonce)
        .env("GITHUB_ENTERPRISE_TOKEN", nonce)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_NO_EXTENSION_UPDATE_NOTIFIER", "1")
        .env("GHD_PASSTHROUGH", "1")
        .env("HTTP_PROXY", "http://127.0.0.1:0")
        .env("HTTPS_PROXY", "http://127.0.0.1:0")
        .env("ALL_PROXY", "http://127.0.0.1:0")
        .env("NO_PROXY", "127.0.0.1,localhost")
}
