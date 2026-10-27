//! Resolve one GitHub credential/host identity before daemon discovery.
//!
//! Tokens stay in memory and the child environment. Only a domain-separated
//! cryptographic digest enters paths or the loopback identity header.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Identity {
    host: String,
    token: String,
    key: String,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

pub fn hostname(value: &str) -> io::Result<String> {
    let host = value.to_ascii_lowercase();
    if host.is_empty()
        || host.len() > 253
        || host.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
    {
        return Err(invalid("invalid GitHub hostname"));
    }
    Ok(host)
}

impl Identity {
    pub fn resolve(host: Option<&str>) -> io::Result<Self> {
        Self::resolve_with(
            host,
            |name| std::env::var(name).ok(),
            |host| {
                let output = Command::new("gh")
                    .args(["auth", "token", "--hostname", host])
                    .output()?;
                if !output.status.success() {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "no GitHub credential for selected host",
                    ));
                }
                String::from_utf8(output.stdout).map_err(|_| invalid("credential is not UTF-8"))
            },
        )
    }

    fn resolve_with(
        selected: Option<&str>,
        env: impl Fn(&str) -> Option<String>,
        fallback: impl FnOnce(&str) -> io::Result<String>,
    ) -> io::Result<Self> {
        let host = hostname(
            selected
                .or(env("GH_HOST").as_deref())
                .unwrap_or("github.com"),
        )?;
        let public = host == "github.com" || host.ends_with(".ghe.com");
        let names = if public {
            ["GH_TOKEN", "GITHUB_TOKEN"]
        } else {
            ["GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN"]
        };
        let token = match names
            .into_iter()
            .find_map(|name| env(name).filter(|v| !v.is_empty()))
        {
            Some(token) => token,
            None => fallback(&host)?.trim().to_string(),
        };
        Self::from_token(&host, token)
    }

    /// Construct an explicitly host-bound identity for embedded callers.
    pub fn from_token(host: &str, token: String) -> io::Result<Self> {
        let host = hostname(host)?;
        if token.is_empty()
            || token
                .bytes()
                .any(|c| c.is_ascii_whitespace() || c.is_ascii_control())
        {
            return Err(invalid("missing or invalid GitHub credential"));
        }
        let key =
            kernal_api::hash::blake3_bytes(format!("ghd-identity-v1\0{host}\0{token}").as_bytes())
                .to_hex();
        Ok(Self { host, token, key })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn directory(&self, base: &Path) -> PathBuf {
        base.join("identities-v1").join(&self.key)
    }

    /// Pin the resolved credential when starting a daemon, even if gh's
    /// stored active account rotates between lookup and child execution.
    pub fn configure_child(&self, command: &mut Command) {
        command.env("GH_HOST", &self.host);
        command.env("GH_TOKEN", &self.token);
        command.env("GH_ENTERPRISE_TOKEN", &self.token);
    }

    pub fn api_base(&self) -> String {
        if self.host == "github.com" {
            "https://api.github.com".into()
        } else if self.host.ends_with(".ghe.com") {
            format!("https://api.{}", self.host)
        } else {
            format!("https://{}/api/v3", self.host)
        }
    }

    pub fn graphql_url(&self) -> String {
        if self.host == "github.com" || self.host.ends_with(".ghe.com") {
            format!("{}/graphql", self.api_base())
        } else {
            format!("https://{}/api/graphql", self.host)
        }
    }
}

/// A repository selector may bind its host explicitly. Never discard it.
pub fn repository(value: &str) -> io::Result<(Option<String>, String, String)> {
    let value = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("ssh://git@"))
        .or_else(|| {
            value
                .strip_prefix("git@")
                .and_then(|s| s.split_once(':').map(|_| s))
        })
        .unwrap_or(value);
    let normalized = value.replacen(':', "/", 1);
    let parts: Vec<_> = normalized
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .split('/')
        .collect();
    let (host, owner, repo) = match parts.as_slice() {
        [owner, repo] => (None, *owner, *repo),
        [host, owner, repo] => (Some(hostname(host)?), *owner, *repo),
        _ => {
            return Err(invalid(
                "repository must be owner/repo or hostname/owner/repo",
            ));
        }
    };
    if !crate::fields::valid_name(owner) || !crate::fields::valid_name(repo) {
        return Err(invalid("invalid repository owner/name"));
    }
    Ok((host, owner.into(), repo.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(host: Option<&str>, values: &[(&str, &str)]) -> Identity {
        Identity::resolve_with(
            host,
            |key| {
                values
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, v)| (*v).into())
            },
            |_| Err(invalid("no fallback")),
        )
        .expect("credential")
    }

    #[test]
    fn credential_precedence_matches_selected_host() {
        let values = [
            ("GH_TOKEN", "public-first"),
            ("GITHUB_TOKEN", "public-second"),
            ("GH_ENTERPRISE_TOKEN", "enterprise-first"),
            ("GITHUB_ENTERPRISE_TOKEN", "enterprise-second"),
        ];
        assert_eq!(resolve(None, &values).token(), "public-first");
        assert_eq!(
            resolve(Some("octo.ghe.com"), &values).token(),
            "public-first"
        );
        assert_eq!(
            resolve(Some("git.example.com"), &values).token(),
            "enterprise-first"
        );
        assert_eq!(
            resolve(
                None,
                &[
                    ("GH_HOST", "git.example.com"),
                    ("GITHUB_ENTERPRISE_TOKEN", "enterprise-second")
                ]
            )
            .token(),
            "enterprise-second"
        );
        assert_eq!(
            resolve(None, &[("GH_TOKEN", ""), ("GITHUB_TOKEN", "second")]).token(),
            "second"
        );
    }

    #[test]
    fn fallback_is_host_bound_and_missing_identity_is_an_error() {
        let identity = Identity::resolve_with(
            Some("GIT.EXAMPLE.COM"),
            |_| None,
            |host| {
                assert_eq!(host, "git.example.com");
                Ok("stored\n".into())
            },
        )
        .expect("stored credential");
        assert_eq!(identity.token(), "stored");
        assert!(Identity::resolve_with(None, |_| None, |_| Ok(String::new())).is_err());
        assert!(Identity::resolve_with(None, |_| None, |_| Err(invalid("missing"))).is_err());
    }

    #[test]
    fn identities_have_private_stable_distinct_keys() {
        let a = resolve(None, &[("GH_TOKEN", "a")]);
        let b = resolve(None, &[("GH_TOKEN", "b")]);
        let other = resolve(Some("other.example"), &[("GH_ENTERPRISE_TOKEN", "a")]);
        assert_eq!(
            a.key(),
            resolve(Some("GITHUB.COM"), &[("GH_TOKEN", "a")]).key()
        );
        assert_ne!(a.key(), b.key());
        assert_ne!(a.key(), other.key());
        assert_eq!(a.key().len(), 64);
        assert!(a.key().bytes().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn host_and_repository_inputs_cannot_inject_urls_or_paths() {
        for bad in [
            "",
            "https://github.com",
            "github.com:443",
            "x/y",
            "a@b",
            "a..b",
            "-a",
            "a\nb",
        ] {
            assert!(hostname(bad).is_err(), "{bad:?}");
        }
        for value in [
            "git@git.example.com:o/r.git",
            "ssh://git@git.example.com/o/r.git",
            "https://git.example.com/o/r",
            "git.example.com/o/r",
        ] {
            assert_eq!(
                repository(value).expect("repo"),
                (Some("git.example.com".into()), "o".into(), "r".into())
            );
        }
        assert!(repository("o/r/extra/path").is_err());
        assert!(repository("o/../r").is_err());
    }

    #[test]
    fn api_urls_follow_github_deployment() {
        let public = resolve(None, &[("GH_TOKEN", "a")]);
        let enterprise = resolve(Some("git.example.com"), &[("GH_ENTERPRISE_TOKEN", "a")]);
        assert_eq!(public.graphql_url(), "https://api.github.com/graphql");
        assert_eq!(enterprise.api_base(), "https://git.example.com/api/v3");
        assert_eq!(
            enterprise.graphql_url(),
            "https://git.example.com/api/graphql"
        );
    }
}
