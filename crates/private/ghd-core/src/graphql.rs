//! `ghd api graphql`: gh's `-f`/`-F` argument shape, sent to the daemon.

use crate::client;
use crate::proto::{GraphqlRequest, GraphqlResponse, GraphqlVariable};
use prost::Message;
use std::io::{self, Write};

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// Parse `api graphql` arguments, without touching the network. `read_file`
/// resolves gh's `-F name=@file` form.
pub fn parse(
    args: &[&str],
    read_file: impl Fn(&str) -> io::Result<String>,
) -> io::Result<GraphqlRequest> {
    let _ = (args, read_file);
    Err(invalid("not implemented".into()))
}

/// `ghd api graphql ...`: print the body; fail (exit 1) when it has errors.
pub fn run(args: &[&str]) -> io::Result<()> {
    let request = parse(args, |path| std::fs::read_to_string(path))?;
    let reply = request_reply(&request)?;
    if std::env::var_os("GHD_DEBUG").is_some() {
        io::stderr().write_all(format!("ghd: {}\n", reply.source).as_bytes())?;
    }
    io::stdout().lock().write_all(&reply.json)?;
    if reply.error.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(reply.error))
    }
}

/// One `api graphql` round trip.
pub fn request_reply(request: &GraphqlRequest) -> io::Result<GraphqlResponse> {
    let (status, body) = client::call("POST", "/v1/graphql", &request.encode_to_vec())?;
    GraphqlResponse::decode(body.as_slice())
        .map_err(|_| io::Error::other(format!("daemon HTTP {status}: undecodable reply")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_file(_: &str) -> io::Result<String> {
        Err(io::Error::other("unused"))
    }

    fn var(name: &str, value: &str, typed: bool) -> GraphqlVariable {
        GraphqlVariable {
            name: name.into(),
            value: value.into(),
            typed,
        }
    }

    #[test]
    fn query_and_fields_follow_gh() {
        let req = parse(
            &["-f", "query=query($n:Int!){x}", "-F", "n=3", "--raw-field", "s=3", "--field", "b=true"],
            no_file,
        )
        .expect("parses");
        assert_eq!(req.query, "query($n:Int!){x}");
        assert_eq!(
            req.variables,
            [var("n", "3", true), var("s", "3", false), var("b", "true", true)]
        );
    }

    #[test]
    fn typed_query_and_attached_flag_values_work_too() {
        let req = parse(&["-Fquery={viewer{login}}", "-fo=obs"], no_file).expect("parses");
        assert_eq!(req.query, "{viewer{login}}");
        assert_eq!(req.variables, [var("o", "obs", false)]);
    }

    #[test]
    fn typed_at_file_reads_the_file() {
        let req = parse(&["-F", "query=@q.graphql"], |p| Ok(format!("<{p}>"))).expect("parses");
        assert_eq!(req.query, "<q.graphql>");
    }

    #[test]
    fn a_query_is_required_and_unknown_flags_are_refused() {
        assert!(parse(&["-f", "a=b"], no_file).is_err());
        assert!(parse(&["-f", "query={x}", "--jq", ".data"], no_file).is_err());
        assert!(parse(&["-f", "novalue"], no_file).is_err());
    }
}
