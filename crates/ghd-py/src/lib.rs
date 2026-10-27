//! PyO3 bindings for `ghd`.
//!
//! Thin translation layer only: every function forwards to the amalgam's
//! client side (no `daemon` feature; the bundled `ghd` binary runs the
//! daemon). Lets Python tools such as `pr_merge_watch` read through the
//! cache without spawning a process per call.

use pyo3::exceptions::PyOSError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

#[pyfunction]
fn version_banner() -> String {
    ghd::version_banner()
}

/// `gh pr view <number> -R owner/repo --json <fields>` through the daemon;
/// returns the JSON document gh would print.
#[pyfunction]
#[pyo3(signature = (owner, repo, number, fields, hostname=None))]
fn pr_view(
    owner: String,
    repo: String,
    number: u64,
    fields: Vec<String>,
    hostname: Option<String>,
) -> PyResult<String> {
    let number = number.to_string();
    let selector = format!("{owner}/{repo}");
    let fields = fields.join(",");
    let mut args = vec![
        number.as_str(),
        "-R",
        selector.as_str(),
        "--json",
        fields.as_str(),
    ];
    if let Some(hostname) = hostname.as_deref() {
        args.extend(["--hostname", hostname]);
    }
    let request = ghd::cli::parse_pr_view(&args, || unreachable!("explicit repository"))
        .map_err(|e| PyOSError::new_err(e.to_string()))?;
    let reply =
        ghd::cli::pr_view_request(&request).map_err(|e| PyOSError::new_err(e.to_string()))?;
    Ok(String::from_utf8_lossy(&reply.json).into_owned())
}

type ReadTuple = (Py<PyBytes>, Py<PyBytes>, i32, String, bool);

/// Return stdout/stderr/status without guessing whether a nonzero read (for
/// example pending checks, exit 8) is an exceptional transport failure.
#[pyfunction]
#[pyo3(signature = (args, no_cache=false, ttl_ms=None))]
fn gh_read(
    py: Python<'_>,
    args: Vec<String>,
    no_cache: bool,
    ttl_ms: Option<u64>,
) -> PyResult<ReadTuple> {
    let response = py
        .detach(|| {
            ghd::request_policy::scope(
                ghd::request_policy::Policy {
                    bypass: no_cache,
                    ttl_ms,
                },
                || ghd::reads::read(&args),
            )
        })
        .map_err(|error| PyOSError::new_err(error.to_string()))?;
    Ok((
        PyBytes::new(py, &response.stdout).unbind(),
        PyBytes::new(py, &response.stderr).unbind(),
        response.exit_code,
        response.source,
        response.limit_reached,
    ))
}

type SnapshotTuple = (Py<PyBytes>, i32, String, Option<u64>, bool, bool, String);

#[pyfunction]
fn snapshot(py: Python<'_>, args: Vec<String>) -> PyResult<SnapshotTuple> {
    let response = py
        .detach(|| {
            let spec = ghd::snapshot::parse(
                &args,
                std::env::var("GH_REPO").ok().as_deref(),
                &std::env::var("GH_HOST").unwrap_or_else(|_| "github.com".into()),
            )?;
            ghd::snapshot::request(&spec)
        })
        .map_err(|error: std::io::Error| PyOSError::new_err(error.to_string()))?;
    Ok((
        PyBytes::new(py, &response.json).unbind(),
        response.exit_code,
        response.source,
        response.age_ms,
        response.stale,
        response.truncated,
        response.error,
    ))
}

#[pymodule]
fn _native(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(version_banner, module)?)?;
    module.add_function(wrap_pyfunction!(pr_view, module)?)?;
    module.add_function(wrap_pyfunction!(gh_read, module)?)?;
    module.add_function(wrap_pyfunction!(snapshot, module)?)?;
    Ok(())
}
