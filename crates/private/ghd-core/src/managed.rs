//! Explicit verified-tool publication and opt-in managed-gh discovery.
use crate::{durable, platform};
use kernal_api::json::{self, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    time::Duration,
};
const MAX_BINARY: usize = 64 * 1024 * 1024;
const LAUNCHERS: &[&str] = &["gh", "gh.cmd", "install.sh", "install.cmd", "bootstrap.py"];
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid managed tool record or artifact",
    )
}
fn text<'a>(record: &'a BTreeMap<String, Value>, key: &str) -> io::Result<&'a str> {
    match record.get(key) {
        Some(Value::String(value)) => Ok(value),
        _ => Err(invalid()),
    }
}
fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn valid_kind(kind: &str) -> io::Result<()> {
    if matches!(kind, "gh" | "ghd") {
        Ok(())
    } else {
        Err(invalid())
    }
}
fn record(root: &Path, kind: &str) -> io::Result<BTreeMap<String, Value>> {
    valid_kind(kind)?;
    let bytes = platform::fs::read_private_file(&root.join(format!("{kind}.json")), 4096)?;
    let Value::Object(record) = json::parse(&bytes).map_err(io::Error::other)? else {
        return Err(invalid());
    };
    if record.get("schema") != Some(&Value::Unsigned(1))
        && record.get("schema") != Some(&Value::Signed(1))
    {
        return Err(invalid());
    }
    if text(&record, "kind")? != kind
        || !digest(text(&record, "digest")?)
        || !digest(text(&record, "binary_sha256")?)
        || !digest(text(&record, "artifact_sha256")?)
    {
        return Err(invalid());
    }
    Ok(record)
}
fn binary(root: &Path, kind: &str, hash: &str) -> PathBuf {
    root.join("objects")
        .join(format!("{kind}-{hash}{}", platform::host::exe_suffix()))
}
pub fn installed(root: &Path, kind: &str) -> io::Result<PathBuf> {
    if !root.is_absolute() {
        return Err(invalid());
    }
    let record = record(root, kind)?;
    let expected = text(&record, "digest")?;
    let path = binary(root, kind, expected);
    let bytes = platform::fs::read_private_file(&path, MAX_BINARY)?;
    if kernal_api::hash::blake3_bytes(&bytes).to_hex() != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "managed binary checksum mismatch",
        ));
    }
    if is_shim(&path)? {
        return Err(invalid());
    }
    Ok(path)
}
pub fn selected_gh() -> io::Result<Option<PathBuf>> {
    std::env::var_os("GHD_MANAGED_GH_DIR")
        .map(|root| installed(&PathBuf::from(root), "gh"))
        .transpose()
}
pub fn is_shim(path: &Path) -> io::Result<bool> {
    if !path.metadata()?.is_file() {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(1024).read_to_end(&mut bytes)?;
    Ok(bytes
        .windows(b"ghd-agent-shim".len())
        .any(|v| v == b"ghd-agent-shim"))
}
fn version_valid(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 64
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}
fn probe(path: &Path, kind: &str, version: &str) -> io::Result<()> {
    let output = kernal_api::run_bounded_command(
        kernal_api::SpawnSpec::new(path)
            .arg("--version")
            .stdin(kernal_api::StreamMode::Null),
        Duration::from_secs(5),
        65536,
    )
    .map_err(io::Error::other)?;
    let banner = String::from_utf8_lossy(&output.stdout);
    let words: Vec<_> = banner.split_whitespace().take(3).collect();
    let matches = if kind == "ghd" {
        words.first() == Some(&"ghd") && words.get(1) == Some(&version)
    } else {
        words.first() == Some(&"gh")
            && words.get(1) == Some(&"version")
            && words.get(2) == Some(&version)
    };
    if !output.exit.is_success() || !matches {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "artifact does not match the pinned tool/version",
        ));
    }
    Ok(())
}
fn publish(
    root: &Path,
    kind: &str,
    source: &Path,
    version: &str,
    artifact: &str,
    sha256: &str,
) -> io::Result<()> {
    valid_kind(kind)?;
    if !root.is_absolute() || !version_valid(version) || !digest(artifact) || !digest(sha256) {
        return Err(invalid());
    }
    platform::fs::create_private_dir(root)?;
    let (_lock, _) = durable::lock_directory(root)?;
    for entry in fs::read_dir(root)? {
        if entry?
            .file_name()
            .to_string_lossy()
            .starts_with(".tool-stage-")
        {
            return Err(io::Error::other(
                "interrupted tool staging requires recovery before publication",
            ));
        }
    }
    let objects = root.join("objects");
    platform::fs::create_private_dir(&objects)?;
    if !source.metadata()?.is_file() {
        return Err(invalid());
    }
    let file = fs::File::open(source)?;
    if !file.metadata()?.is_file() {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BINARY as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > MAX_BINARY {
        return Err(invalid());
    }
    if kernal_api::hash::sha256_bytes(&bytes).to_hex() != sha256.to_ascii_lowercase() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "binary SHA256 mismatch",
        ));
    }
    let active = match record(root, kind) {
        Ok(record) => Some(binary(root, kind, text(&record, "digest")?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    cleanup(root, kind, active.as_deref())?;
    let hash = kernal_api::hash::blake3_bytes(&bytes).to_hex();
    let staged = kernal_api::platform::fs::TemporaryDirectory::in_directory(root, ".tool-stage-")?;
    platform::fs::create_private_dir(staged.path())?;
    let stage_binary = staged
        .path()
        .join(format!("{kind}{}", platform::host::exe_suffix()));
    platform::fs::write_private(&stage_binary, &bytes)?;
    kernal_api::platform::fs::make_owner_executable(&stage_binary)?;
    if is_shim(&stage_binary)? {
        return Err(invalid());
    }
    probe(&stage_binary, kind, version)?;
    let target = binary(root, kind, &hash);
    if target.try_exists()? {
        let existing = platform::fs::read_private_file(&target, MAX_BINARY)?;
        if existing != bytes {
            return Err(invalid());
        }
    } else {
        fs::rename(&stage_binary, &target)?;
    }
    let doc = Value::Object(BTreeMap::from([
        ("schema".into(), Value::Unsigned(1)),
        ("kind".into(), Value::String(kind.into())),
        ("version".into(), Value::String(version.into())),
        ("digest".into(), Value::String(hash)),
        (
            "artifact_sha256".into(),
            Value::String(artifact.to_ascii_lowercase()),
        ),
        (
            "binary_sha256".into(),
            Value::String(sha256.to_ascii_lowercase()),
        ),
    ]));
    platform::fs::write_private(
        &root.join(format!("{kind}.json")),
        &json::encode(&doc, json::Layout::Compact).map_err(io::Error::other)?,
    )?;
    staged.close()?;
    cleanup(root, kind, Some(&target))?;
    Ok(())
}
fn cleanup(root: &Path, kind: &str, keep: Option<&Path>) -> io::Result<()> {
    let objects = root.join("objects");
    if !objects.try_exists()? {
        return Ok(());
    }
    for entry in fs::read_dir(objects)? {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or_else(invalid)?;
        let owned = name
            .strip_prefix(&format!("{kind}-"))
            .and_then(|v| v.strip_suffix(platform::host::exe_suffix()))
            .is_some_and(digest);
        if owned && Some(path.as_path()) != keep {
            platform::fs::read_private_file(&path, MAX_BINARY)?;
            fs::remove_file(path)?;
        }
    }
    Ok(())
}
fn delegate(root: &Path, words: &[&str]) -> io::Result<()> {
    if !root.is_absolute() {
        return Err(invalid());
    }
    // Read custody before creating/acquiring a lock: runtime never repairs an
    // untrusted installation and never executes its own unchecked validator.
    record(root, "ghd")?;
    let (lock, _) = durable::lock_directory(root)?;
    let path = installed(root, "ghd")?;
    let mut command = std::process::Command::new(&path);
    command.args(words).env("GHD_EXE", &path);
    if std::env::var_os("GHD_MANAGED_GH_DIR").is_none() && root.join("gh.json").try_exists()? {
        command.env("GHD_MANAGED_GH_DIR", root);
    }
    let mut child = command.spawn()?;
    drop(lock);
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(crate::compat::exit_error(
            platform::process::exit_status_code(status),
        ))
    }
}
fn launchers(root: &Path, destination: &Path, source: &Path) -> io::Result<()> {
    if !destination.is_absolute() || !source.is_absolute() {
        return Err(invalid());
    }
    installed(root, "ghd")?;
    let canonical_root = root.canonicalize()?;
    let canonical_destination = if destination.try_exists()? {
        destination.canonicalize()?
    } else {
        destination
            .parent()
            .ok_or_else(invalid)?
            .canonicalize()?
            .join(destination.file_name().ok_or_else(invalid)?)
    };
    if canonical_destination.starts_with(&canonical_root) {
        return Err(io::Error::other(
            "launchers must be trusted separately, outside managed root",
        ));
    }
    let marker = destination.join(".ghd-launchers-v1");
    if destination.try_exists()?
        && fs::read_dir(destination)?.next().is_some()
        && platform::fs::read_private_file(&marker, 64)? != b"ghd agent launchers v1\n"
    {
        return Err(invalid());
    }
    let mut templates = Vec::new();
    for &name in LAUNCHERS {
        let path = source.join(name);
        if !path.metadata()?.is_file() {
            return Err(invalid());
        }
        let mut body = Vec::new();
        fs::File::open(path)?.take(65537).read_to_end(&mut body)?;
        if body.len() > 65536
            || !body
                .windows(b"ghd-agent-shim".len())
                .any(|v| v == b"ghd-agent-shim")
        {
            return Err(invalid());
        }
        templates.push((name, body));
    }
    platform::fs::create_private_dir(destination)?;
    let (_lock, _) = durable::lock_directory(destination)?;
    for (name, _) in &templates {
        let path = destination.join(name);
        if path.try_exists()? {
            let current = platform::fs::read_private_file(&path, 65536)?;
            if !current
                .windows(b"ghd-agent-shim".len())
                .any(|v| v == b"ghd-agent-shim")
            {
                return Err(invalid());
            }
        }
    }
    // Ownership permits a bounded retry after an interrupted materialization;
    // each individual launcher still publishes atomically.
    platform::fs::write_private(&marker, b"ghd agent launchers v1\n")?;
    for (name, body) in templates {
        let path = destination.join(name);
        platform::fs::write_private(&path, &body)?;
        kernal_api::platform::fs::make_owner_executable(&path)?;
    }
    Ok(())
}
fn remove_launchers(destination: &Path) -> io::Result<()> {
    if !destination.is_absolute() {
        return Err(invalid());
    }
    let marker = destination.join(".ghd-launchers-v1");
    if platform::fs::read_private_file(&marker, 64)? != b"ghd agent launchers v1\n" {
        return Err(invalid());
    }
    let (_lock, _) = durable::lock_directory(destination)?;
    let mut owned = Vec::new();
    for name in LAUNCHERS {
        let path = destination.join(name);
        if path.try_exists()? {
            let bytes = platform::fs::read_private_file(&path, 65536)?;
            if !bytes
                .windows(b"ghd-agent-shim".len())
                .any(|v| v == b"ghd-agent-shim")
            {
                return Err(invalid());
            }
            owned.push(path);
        }
    }
    for path in owned {
        fs::remove_file(path)?;
    }
    Ok(())
}
pub fn run(args: &[&str]) -> io::Result<()> {
    match args {
        ["remove-launchers", destination] => remove_launchers(Path::new(destination))?,
        ["run", root, words @ ..] => delegate(Path::new(root), words)?,
        ["launchers", root, destination, source] => {
            launchers(Path::new(root), Path::new(destination), Path::new(source))?
        }
        ["prepare-root", root] => {
            if !Path::new(root).is_absolute() {
                return Err(invalid());
            }
            platform::fs::create_private_dir(Path::new(root))?;
        }
        ["publish", root, kind, source, version, artifact, sha256] => publish(
            Path::new(root),
            kind,
            Path::new(source),
            version.strip_prefix('v').unwrap_or(version),
            artifact,
            sha256,
        )?,
        ["status", root, kind] => {
            let root = Path::new(root);
            match installed(root, kind) {
                Ok(path) => {
                    let mut doc = record(root, kind)?;
                    doc.insert("installed".into(), Value::Bool(true));
                    doc.insert(
                        "path".into(),
                        Value::String(path.to_str().ok_or_else(invalid)?.into()),
                    );
                    println!(
                        "{}",
                        String::from_utf8(
                            json::encode(&Value::Object(doc), json::Layout::Compact)
                                .map_err(io::Error::other)?
                        )
                        .map_err(io::Error::other)?
                    );
                }
                Err(error)
                    if error.kind() == io::ErrorKind::NotFound
                        && !root.join(format!("{kind}.json")).try_exists()? =>
                {
                    println!("{{\"installed\":false}}")
                }
                Err(error) => return Err(error),
            }
        }
        ["uninstall", root, kind] => {
            valid_kind(kind)?;
            let root = Path::new(root);
            if !root.is_absolute() {
                return Err(invalid());
            }
            let (_lock, _) = durable::lock_directory(root)?;
            cleanup(root, kind, None)?;
            let path = root.join(format!("{kind}.json"));
            if path.try_exists()? {
                platform::fs::read_private_file(&path, 4096)?;
                fs::remove_file(path)?;
            }
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "tools prepare-root ROOT | publish ROOT KIND SOURCE VERSION ARTIFACT_SHA256 BINARY_SHA256 | status ROOT KIND | uninstall ROOT KIND",
            ));
        }
    }
    Ok(())
}
