//! PyO3 bindings for `ghd`.
//!
//! Thin translation layer only: every function forwards to the amalgam's
//! client side (no `daemon` feature; the bundled `ghd` binary runs the
//! daemon). Lets Python tools such as `pr_merge_watch` read through the
//! cache without spawning a process per call.

use pyo3::exceptions::PyOSError;
use pyo3::prelude::*;

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

#[pymodule]
fn _native(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(version_banner, module)?)?;
    module.add_function(wrap_pyfunction!(pr_view, module)?)?;
    Ok(())
}
