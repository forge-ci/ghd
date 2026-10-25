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
fn pr_view(owner: String, repo: String, number: u64, fields: Vec<String>) -> PyResult<String> {
    let request = ghd::proto::PrViewRequest {
        owner,
        repo,
        number,
        fields,
    };
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
