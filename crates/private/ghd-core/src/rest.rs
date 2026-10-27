//! Safe REST read grammar; ambiguous or diagnostic shapes remain native gh.

use crate::identity;
use std::io;

#[derive(Clone, Debug)]
pub struct RestPlan {
    pub host: String,
    pub fetch_args: Vec<String>,
    pub jq: Option<String>,
    pub template: Option<String>,
    pub force_refresh: bool,
    pub rate_limit: bool,
}

#[derive(Default)]
struct Parsed {
    endpoint: Option<String>,
    method: Option<String>,
    host: Option<String>,
    fields: bool,
    uncacheable: bool,
    force_refresh: bool,
    retained: Vec<String>,
    jq: Option<String>,
    template: Option<String>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn value_flag(name: &str) -> &str {
    match name {
        "-X" => "--method",
        "-H" => "--header",
        "-f" => "--raw-field",
        "-F" => "--field",
        "-p" => "--preview",
        "-q" => "--jq",
        "-t" => "--template",
        other => other,
    }
}

fn split_flag(word: &str) -> (&str, Option<&str>) {
    if let Some((name, value)) = word.split_once('=')
        && name.starts_with("--")
    {
        return (name, Some(value));
    }
    if word.starts_with('-')
        && word.len() > 2
        && word.as_bytes()[1].is_ascii()
        && ["-X", "-H", "-f", "-F", "-p", "-q", "-t"].contains(&&word[..2])
    {
        return (&word[..2], Some(&word[2..]));
    }
    (word, None)
}

fn parse(args: &[String]) -> io::Result<Option<Parsed>> {
    if args.first().is_none_or(|word| word != "api") {
        return Ok(None);
    }
    let mut parsed = Parsed::default();
    let mut words = args[1..].iter();
    while let Some(word) = words.next() {
        let (name, attached) = split_flag(word);
        let name = value_flag(name);
        match name {
            "--method" | "--hostname" | "--header" | "--field" | "--raw-field" | "--preview"
            | "--jq" | "--template" | "--input" | "--cache" => {
                let value = attached
                    .or_else(|| words.next().map(String::as_str))
                    .ok_or_else(|| invalid("API flag needs a value"))?;
                option(&mut parsed, name, value)?;
            }
            "--paginate"
            | "--slurp"
            | "--include"
            | "-i"
            | "--silent"
            | "--verbose"
            | "--allow-escape-sequences" => parsed.uncacheable = true,
            name if name.starts_with('-') => return Ok(None),
            _ => {
                if parsed.endpoint.replace(word.clone()).is_some() {
                    return Ok(None);
                }
            }
        }
    }
    if parsed.endpoint.is_none() {
        return Ok(None);
    }
    Ok(Some(parsed))
}

fn option(parsed: &mut Parsed, name: &str, value: &str) -> io::Result<()> {
    match name {
        "--method" => {
            if parsed.method.replace(value.into()).is_some() {
                parsed.uncacheable = true;
            }
        }
        "--hostname" => {
            if parsed.host.replace(value.into()).is_some() {
                parsed.uncacheable = true;
            }
        }
        "--jq" => {
            if parsed.jq.replace(value.into()).is_some() {
                parsed.uncacheable = true;
            }
        }
        "--template" => {
            if parsed.template.replace(value.into()).is_some() {
                parsed.uncacheable = true;
            }
        }
        "--header" => {
            let Some((name, value)) = value.split_once(':') else {
                return Err(invalid("API header needs name:value"));
            };
            match name.trim().to_ascii_lowercase().as_str() {
                "authorization"
                | "proxy-authorization"
                | "cookie"
                | "range"
                | "if-range"
                | "if-none-match"
                | "if-modified-since"
                | "if-match"
                | "if-unmodified-since" => parsed.uncacheable = true,
                "pragma" => {
                    parsed.force_refresh |= value.trim().eq_ignore_ascii_case("no-cache");
                }
                "cache-control" => {
                    let directives: Vec<_> = value
                        .split(',')
                        .map(|part| part.trim().split('=').next().unwrap_or("").trim())
                        .collect();
                    parsed.uncacheable |= directives
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case("no-store"));
                    // Conservative request bounds always validate at the origin;
                    // server max-age separately limits retained freshness.
                    parsed.force_refresh |= directives.iter().any(|name| {
                        ["no-cache", "max-age", "min-fresh"]
                            .iter()
                            .any(|known| name.eq_ignore_ascii_case(known))
                    });
                }
                _ => {}
            }
            parsed
                .retained
                .extend(["--header".into(), format!("{name}:{value}")]);
        }
        "--input" => {
            parsed.fields = true;
            parsed.uncacheable = true;
        }
        "--cache" => parsed.uncacheable = true,
        _ => {
            parsed.fields |= matches!(name, "--field" | "--raw-field");
            parsed.uncacheable |= name == "--field"
                && value
                    .split_once('=')
                    .is_some_and(|(_, v)| v.starts_with('@'));
            parsed.retained.extend([name.into(), value.into()]);
        }
    }
    Ok(())
}

impl Parsed {
    fn method(&self) -> &str {
        self.method
            .as_deref()
            .unwrap_or(if self.fields { "POST" } else { "GET" })
    }
    fn graphql(&self) -> bool {
        self.endpoint.as_deref().is_some_and(|endpoint| {
            endpoint.split('?').next().is_some_and(|path| {
                path.trim_matches('/') == "graphql" || path.ends_with("/graphql")
            })
        })
    }
}

/// Used by compatibility dispatch too: a bypassed GET is still a read.
pub fn is_read_only(args: &[&str]) -> bool {
    parse(&args.iter().map(|word| (*word).into()).collect::<Vec<_>>())
        .ok()
        .flatten()
        .is_some_and(|parsed| !parsed.graphql() && matches!(parsed.method(), "GET" | "HEAD"))
}

pub fn prepare(args: &[String], default_host: &str) -> io::Result<Option<RestPlan>> {
    let Some(parsed) = parse(args)? else {
        return Ok(None);
    };
    if parsed.uncacheable
        || parsed.method() != "GET"
        || parsed.graphql()
        || parsed.jq.is_some() && parsed.template.is_some()
    {
        return Ok(None);
    }
    let endpoint = parsed
        .endpoint
        .as_deref()
        .ok_or_else(|| invalid("missing API endpoint"))?;
    if endpoint.contains("://")
        || endpoint.starts_with("//")
        || endpoint.contains('#')
        || endpoint.chars().any(char::is_control)
        || std::iter::once(endpoint)
            .chain(parsed.retained.iter().map(String::as_str))
            .any(|word| {
                ["{owner}", "{repo}", "{branch}"]
                    .iter()
                    .any(|placeholder| word.contains(placeholder))
            })
    {
        return Ok(None);
    }
    let endpoint = endpoint.trim_start_matches('/');
    let host = identity::hostname(parsed.host.as_deref().unwrap_or(default_host))?;
    let rate_limit = endpoint.split('?').next() == Some("rate_limit");
    let mut fetch_args = vec![
        "api".into(),
        endpoint.into(),
        "--method".into(),
        "GET".into(),
        "--hostname".into(),
        host.clone(),
        "--include".into(),
    ];
    fetch_args.extend(parsed.retained);
    Ok(Some(RestPlan {
        host,
        fetch_args,
        jq: parsed.jq,
        template: parsed.template,
        force_refresh: parsed.force_refresh || rate_limit,
        rate_limit,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan(args: &[&str]) -> Option<RestPlan> {
        prepare(
            &args.iter().map(|word| (*word).into()).collect::<Vec<_>>(),
            "github.com",
        )
        .expect("parse")
    }
    #[test]
    fn implicit_post_files_ranges_credentials_and_opaque_shapes_stay_native() {
        for args in [
            vec!["api", "user", "-f", "name=x"],
            vec!["api", "user", "-XGET", "-F", "value=@-"],
            vec!["api", "user", "-H", "Range: bytes=0-9"],
            vec!["api", "user", "-H", "If-Match: \"v1\""],
            vec![
                "api",
                "user",
                "-H",
                "If-Unmodified-Since: Wed, 01 Jan 2025 00:00:00 GMT",
            ],
            vec!["api", "user", "-H", "Authorization: token example"],
            vec!["api", "user", "--paginate"],
            vec!["api", "user", "--include"],
            vec!["api", "repos/{owner}/{repo}"],
            vec!["api", "https://other.example/user"],
            vec!["api", "graphql", "-XGET"],
        ] {
            assert!(plan(&args).is_none(), "{args:?}");
        }
        assert!(!is_read_only(&["api", "user", "-f", "name=x"]));
        assert!(is_read_only(&["api", "user", "--paginate"]));
        assert!(!is_read_only(&["api", "graphql", "-XGET"]));
    }
    #[test]
    fn request_freshness_bounds_never_receive_a_window_hit() {
        for value in [
            "max-age=0",
            "max-age=60",
            "no-cache=\"Set-Cookie\"",
            "min-fresh=10",
        ] {
            let args = [
                "api",
                "user",
                "--header",
                &format!("Cache-Control: {value}"),
            ];
            assert!(plan(&args).expect("safe read").force_refresh);
        }
        assert!(
            plan(&[
                "api",
                "user",
                "--header",
                "Cache-Control: no-store=\"Set-Cookie\""
            ])
            .is_none()
        );
    }

    #[test]
    fn requests_keep_parameters_representations_host_and_fresh_measurements() {
        let a = plan(&[
            "api",
            "/user?a=1",
            "-XGET",
            "-F",
            "b=true",
            "-HAccept: application/json",
            "--hostname",
            "ghe.example",
            "--jq",
            ".id",
        ])
        .expect("read");
        assert_eq!(a.host, "ghe.example");
        assert_eq!(a.jq.as_deref(), Some(".id"));
        assert!(a.fetch_args.contains(&"b=true".into()));
        let other = plan(&["api", "/user?a=2", "-HAccept: application/json"]).expect("other");
        assert_ne!(a.fetch_args, other.fetch_args);
        assert!(
            plan(&["api", "rate_limit"])
                .expect("measurement")
                .force_refresh
        );
        assert!(
            plan(&["api", "user", "-HCache-Control: no-cache"])
                .expect("revalidate")
                .force_refresh
        );
    }
}
