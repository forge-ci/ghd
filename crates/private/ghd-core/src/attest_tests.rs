use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct Repo(std::path::PathBuf);
impl Repo {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ghd-attest-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let repo = Self(path);
        repo.git(&["init", "-q"]);
        repo.git(&["config", "user.name", "Test"]);
        repo.git(&["config", "user.email", "test@example.com"]);
        repo.git(&["config", "commit.gpgSign", "false"]);
        repo.git(&["config", "core.hooksPath", "/dev/null"]);
        std::fs::write(repo.0.join("ci-attestations.yml"), "version: 1\ngates:\n  rust/all/fmt: {lane: lint}\n  rust/all/clippy: {lane: lint}\njobs:\n  ci.yml:lint: [rust/all/fmt]\n").unwrap();
        repo.git(&["add", "."]);
        repo.git(&["commit", "-qm", "parent"]);
        repo.git(&["commit", "--allow-empty", "-qm", "subject\n\nBody text.\n\nSigned-off-by: Test <test@example.com>\nReviewed-by: Reviewer <review@example.com>"]);
        repo
    }
    fn git(&self, args: &[&str]) -> String {
        git(&self.0, args, None).unwrap()
    }
    fn evidence(&self) -> Vec<u8> {
        format!(r#"{{"v":1,"tree":"{}","host":"x86_64-unknown-linux-gnu","soldr":"0.1.0","results":[{{"id":"rust/all/fmt","ok":true,"secs":12,"key":"abcd"}},{{"id":"rust/all/clippy","ok":true,"secs":3,"key":"ffff"}},{{"id":"rust/all/test","ok":false,"secs":1,"key":"eeee"}}],"autofixed":[]}}"#, self.git(&["rev-parse", "HEAD^{tree}"]).trim()).into_bytes()
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        // Test fixtures contain only disposable commits and files.
        let _ = Command::new("safe-rm")
            .arg("-r")
            .arg(&self.0)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[test]
fn stamps_successes_and_preserves_message_and_existing_trailers() {
    let repo = Repo::new();
    let tree = repo.git(&["rev-parse", "HEAD^{tree}"]);
    let parents = repo.git(&["log", "-1", "--format=%P"]);
    let emitted = stamp(&repo.0, &repo.evidence(), "lint", false).unwrap();
    assert_eq!(emitted.lines().count(), 2);
    assert!(!emitted.contains("rust/all/test"));
    assert_eq!(repo.git(&["rev-parse", "HEAD^{tree}"]), tree);
    assert_eq!(repo.git(&["log", "-1", "--format=%P"]), parents);
    let message = repo.git(&["log", "-1", "--format=%B"]);
    assert!(message.starts_with("subject\n\nBody text.\n\nSigned-off-by:"));
    assert!(message.find("Reviewed-by:").unwrap() < message.find("Ci-Attestation:").unwrap());
    let parsed = git(
        &repo.0,
        &["interpret-trailers", "--parse"],
        Some(message.as_bytes()),
    )
    .unwrap();
    assert!(parsed.contains("Signed-off-by:"));
    assert!(parsed.contains("Reviewed-by:"));
    assert_eq!(parsed.matches("Ci-Attestation:").count(), 2);
}

#[test]
fn refuses_tree_mismatch() {
    let repo = Repo::new();
    let evidence = String::from_utf8(repo.evidence())
        .unwrap()
        .replace(repo.git(&["rev-parse", "HEAD^{tree}"]).trim(), "wrong-tree");
    assert!(
        stamp(&repo.0, evidence.as_bytes(), "lint", false)
            .unwrap_err()
            .to_string()
            .contains("evidence tree does not match")
    );
}

#[test]
fn refuses_dirty_tree_including_untracked_files() {
    let repo = Repo::new();
    let evidence = repo.evidence();
    for file in ["untracked", "ci-attestations.yml"] {
        std::fs::write(repo.0.join(file), "dirty").unwrap();
        assert!(
            stamp(&repo.0, &evidence, "lint", false)
                .unwrap_err()
                .to_string()
                .contains("working tree is dirty")
        );
        repo.git(&["add", file]);
        assert!(
            stamp(&repo.0, &evidence, "lint", false)
                .unwrap_err()
                .to_string()
                .contains("working tree is dirty")
        );
        repo.git(&["commit", "-qm", "fixture change"]);
    }
}

#[test]
fn dry_run_leaves_head_unchanged() {
    let repo = Repo::new();
    let head = repo.git(&["rev-parse", "HEAD"]);
    assert_eq!(
        stamp(&repo.0, &repo.evidence(), "lint", true)
            .unwrap()
            .lines()
            .count(),
        2
    );
    assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
}

#[test]
fn rejects_unsupported_schema_and_no_successes() {
    let repo = Repo::new();
    for (from, to, expected) in [
        ("\"v\":1", "\"v\":2", "schema"),
        ("\"ok\":true", "\"ok\":false", "no successful results"),
    ] {
        let input = String::from_utf8(repo.evidence())
            .unwrap()
            .replace(from, to);
        assert!(
            stamp(&repo.0, input.as_bytes(), "lint", false)
                .unwrap_err()
                .to_string()
                .contains(expected)
        );
    }
}

#[test]
fn canonical_uses_python_ascii_escaping() {
    assert_eq!(
        canonical(&Value::String("é😀\n".into())).unwrap(),
        r#""\u00e9\ud83d\ude00\n""#
    );
}

#[test]
#[ignore = "requires CI_LINT_CHECKOUT pointing at ci.yml commit 24362f3e"]
fn real_verifier_round_trip() {
    let checkout = std::env::var("CI_LINT_CHECKOUT").expect("CI_LINT_CHECKOUT required");
    let pin = git(Path::new(&checkout), &["rev-parse", "HEAD"], None).unwrap();
    assert!(pin.starts_with("24362f3e"), "wrong verifier pin: {pin}");
    let repo = Repo::new();
    let trailers = stamp(&repo.0, &repo.evidence(), "lint", false).unwrap();
    println!("{trailers}");
    let output = Command::new("python3")
        .args(["-P", "-m", "ci_lint", "attest", "verify", "--repo"])
        .arg(&repo.0)
        .args(["--commit", "HEAD"])
        .env("PYTHONPATH", checkout)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    println!(
        "python3 -P -m ci_lint attest verify --repo {} --commit HEAD\n{stdout}{}",
        repo.0.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success());
    assert!(stdout.contains("rust/all/fmt") && stdout.contains("ok"));
}
