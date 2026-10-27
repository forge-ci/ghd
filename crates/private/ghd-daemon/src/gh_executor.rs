//! Bounded, identity-pinned real-gh execution. No shell or ambient auth reuse.

use ghd_core::proto::{GhReadRequest, GhReadResponse};
use ghd_core::reads::CONTEXT_ENV;
use kernal_api::{SpawnSpec, StreamMode};
use std::collections::BTreeSet;
use std::io;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

pub fn validate(request: &GhReadRequest) -> io::Result<()> {
    if request.args.len() > 1024
        || request.args.iter().map(String::len).sum::<usize>() > 1024 * 1024
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "gh read arguments exceed limits",
        ));
    }
    if !Path::new(&request.program).is_absolute() || !Path::new(&request.cwd).is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "gh read paths must be absolute",
        ));
    }
    let metadata = std::fs::metadata(&request.program)?;
    let stamp = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    if !metadata.is_file() || format!("{}:{stamp}", metadata.len()) != request.program_stamp {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "real gh changed; retry with its current executable",
        ));
    }
    if std::fs::canonicalize(&request.program)? == std::env::current_exe()?.canonicalize()? {
        return Err(io::Error::other("real gh resolves recursively to ghd"));
    }
    let mut seen = BTreeSet::new();
    for variable in &request.environment {
        if !CONTEXT_ENV.contains(&variable.name.as_str())
            || !seen.insert(&variable.name)
            || variable.value.len() > 64 * 1024
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported or repeated gh context variable",
            ));
        }
    }
    Ok(())
}

pub fn spec(request: &GhReadRequest, host: &str, token: &str, args: &[String]) -> SpawnSpec {
    let mut spec = SpawnSpec::new(&request.program)
        .args(args.iter().cloned())
        .clear_env(true)
        .current_dir(&request.cwd)
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
        spec = spec.env(&variable.name, &variable.value);
    }
    spec.env("GH_HOST", host)
        .env("GH_TOKEN", token)
        .env("GITHUB_TOKEN", token)
        .env("GH_ENTERPRISE_TOKEN", token)
        .env("GITHUB_ENTERPRISE_TOKEN", token)
        .env("GHD_PASSTHROUGH", "1")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_NO_EXTENSION_UPDATE_NOTIFIER", "1")
}

#[cfg(test)]
pub async fn execute(
    spec: SpawnSpec,
    permit: kernal_api::async_engine::SemaphorePermit,
) -> io::Result<GhReadResponse> {
    run(spec, permit, None, false).await
}

pub(crate) async fn governed(
    spec: SpawnSpec,
    permit: kernal_api::async_engine::SemaphorePermit,
    dispatch: crate::governance::Dispatch,
    included_headers: bool,
) -> io::Result<GhReadResponse> {
    run(spec, permit, Some(dispatch), included_headers).await
}

async fn run(
    spec: SpawnSpec,
    permit: kernal_api::async_engine::SemaphorePermit,
    mut dispatch: Option<crate::governance::Dispatch>,
    included_headers: bool,
) -> io::Result<GhReadResponse> {
    let output = kernal_api::async_engine::launch_blocking(move || {
        // Both process and governor capacity belong to the child until reaped.
        let _permit = permit;
        if let Some(dispatch) = &mut dispatch {
            dispatch.start();
        }
        let output =
            kernal_api::run_bounded_command(spec, Duration::from_secs(60), 32 * 1024 * 1024)
                .map_err(io::Error::other)?;
        if let Some(dispatch) = dispatch {
            if included_headers
                && let Ok((status, _, body)) =
                    crate::run_collection::included_response(&output.stdout)
            {
                let headers =
                    std::str::from_utf8(&output.stdout[..output.stdout.len() - body.len()])
                        .map_err(io::Error::other)?;
                dispatch.complete(status, headers, body)?;
            } else {
                // Opaque porcelain exposes no provider headers or point cost.
                dispatch.complete(0, "", &[])?;
            }
        }
        Ok::<_, io::Error>(output)
    })
    .await
    .map_err(io::Error::other)??;
    let exit_code = crate::platform::process::bounded_exit_code(output.exit.raw_code())?;
    Ok(GhReadResponse {
        stdout: output.stdout,
        stderr: output.stderr,
        exit_code,
        source: "fetched".into(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernal_api::async_engine::{self, Semaphore};

    #[test]
    fn capacity_fixture_child() {
        let Some(directory) = std::env::var_os("GHD_CAPACITY_FIXTURE") else {
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        std::fs::write(directory.join("started"), b"ready").expect("started");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !directory.join("release").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "release child before deadline"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn canceled_executor_retains_capacity_until_its_child_is_reaped() {
        let runtime = async_engine::RuntimeBuilder::multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.run(async {
            for with_governor in [false, true] {
                let directory = std::env::temp_dir().join(format!(
                    "ghd-capacity-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("clock")
                        .as_nanos()
                ));
                std::fs::create_dir(&directory).expect("new directory");
                let capacity = Semaphore::new(1);
                let permit = capacity.acquire().await;
                let spec = SpawnSpec::new(std::env::current_exe().expect("test binary"))
                    .args([
                        "--exact",
                        "gh_executor::tests::capacity_fixture_child",
                        "--nocapture",
                    ])
                    .env("GHD_CAPACITY_FIXTURE", &directory)
                    .stdin(StreamMode::Null);
                let control = crate::governance::Control::at(
                    directory.join("governor"),
                    "capacity-fixture".into(),
                    0,
                );
                let pending = if with_governor {
                    let dispatch = control
                        .enter("unknown", false, false, crate::governance::Event::Process)
                        .await
                        .expect("governor permit");
                    async_engine::launch(governed(spec, permit, dispatch, false))
                } else {
                    async_engine::launch(execute(spec, permit))
                };
                let started = async_engine::timeout(Duration::from_secs(5), async {
                    while !directory.join("started").exists() {
                        async_engine::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await;
                if started.is_err() {
                    std::fs::write(directory.join("release"), b"release")
                        .expect("release slow child");
                }
                started.expect("child running");
                pending.cancel();
                let _ = pending.await;
                let prematurely_available =
                    async_engine::timeout(Duration::from_millis(50), capacity.acquire()).await;
                let retained = prematurely_available.is_err();
                let governor_retained = !with_governor || control.available_for_test() == (3, 2);
                drop(prematurely_available);
                std::fs::write(directory.join("release"), b"release").expect("release child");
                let permit = async_engine::timeout(Duration::from_secs(5), capacity.acquire())
                    .await
                    .expect("child reaped");
                drop(permit);
                let governor_released = !with_governor || control.available_for_test() == (4, 3);
                std::fs::remove_dir_all(directory).expect("clean fixture");
                assert!(
                    retained,
                    "cancellation must not release a live child's permit"
                );
                assert!(
                    governor_retained && governor_released,
                    "governor capacity belongs to the child until reaped"
                );
            }
        });
    }
}
