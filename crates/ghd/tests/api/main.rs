//! The amalgam's one integration test target (`ghd:test:api`), run once
//! per `[rust] ship` feature set.

#[test]
fn version_banner_is_reexported_and_names_ghd() {
    assert!(ghd::version_banner().starts_with("ghd "));
}

#[test]
fn client_surface_is_reexported() {
    assert!(ghd::fields::PR_FIELDS.contains(&"state"));
    assert!(ghd::endpoint::parse("1 t").is_ok());
}

#[cfg(feature = "daemon")]
#[test]
fn daemon_feature_reexports_run_daemon() {
    let run: fn() -> std::io::Result<()> = ghd::run_daemon;
    let _ = run;
}
