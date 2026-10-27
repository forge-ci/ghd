//! Optional bounded owner-private JSON configuration, overridden by environment.
use crate::platform;
use kernal_api::json::{self, Value};
use std::{collections::BTreeMap, io, path::PathBuf};
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "invalid GHD_CONFIG: expected known GHD settings and nonnegative integer values",
    )
}
pub fn parse(bytes: &[u8]) -> io::Result<BTreeMap<String, String>> {
    let Value::Object(values) = json::parse(bytes).map_err(|_| invalid())? else {
        return Err(invalid());
    };
    let mut result = BTreeMap::new();
    for (key, value) in values {
        let text = if key == "GHD_CACHE_TTLS" {
            let encoded = json::encode(&value, json::Layout::Compact).map_err(|_| invalid())?;
            let text = String::from_utf8(encoded).map_err(|_| invalid())?;
            crate::cache_policy::Policy::parse(Some(&text), None)?;
            text
        } else if matches!(
            key.as_str(),
            "GHD_FRESH_MS"
                | "GHD_CACHE_MAX_ENTRIES"
                | "GHD_CACHE_MAX_BYTES"
                | "GHD_RATE_RESERVE"
                | "GHD_CACHE_SETTLED_MS"
                | "GHD_CACHE_DISK"
        ) {
            let number = match value {
                Value::Unsigned(number) => number,
                Value::Signed(number) => u64::try_from(number).map_err(|_| invalid())?,
                _ => return Err(invalid()),
            };
            if key == "GHD_CACHE_DISK" && number > 1 {
                return Err(invalid());
            }
            number.to_string()
        } else {
            return Err(invalid());
        };
        result.insert(key, text);
    }
    Ok(result)
}
pub fn value(name: &str) -> io::Result<Option<String>> {
    let values = match std::env::var_os("GHD_CONFIG") {
        None => BTreeMap::new(),
        Some(path) => {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err(invalid());
            }
            parse(&platform::fs::read_private_file(&path, 16384)?)?
        }
    };
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(values.get(name).cloned()),
        Err(_) => Err(invalid()),
    }
}

pub fn validate() -> io::Result<()> {
    for name in [
        "GHD_FRESH_MS",
        "GHD_CACHE_MAX_ENTRIES",
        "GHD_CACHE_MAX_BYTES",
        "GHD_RATE_RESERVE",
        "GHD_CACHE_SETTLED_MS",
        "GHD_CACHE_DISK",
    ] {
        if let Some(value) = value(name)? {
            let number = value.parse::<u64>().map_err(|_| invalid())?;
            if name == "GHD_CACHE_DISK" && number > 1 {
                return Err(invalid());
            }
        }
    }
    crate::cache_policy::Policy::from_environment()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_is_typed_and_rejects_unknown_or_unsafe_settings() {
        let settings =
            parse(br#"{"GHD_FRESH_MS":40,"GHD_CACHE_DISK":1,"GHD_CACHE_TTLS":{"issue view":10}}"#)
                .expect("config");
        assert_eq!(settings.get("GHD_FRESH_MS").map(String::as_str), Some("40"));
        for bad in [
            br#"{"GH_TOKEN":"secret"}"#.as_slice(),
            br#"{"GHD_FRESH_MS":-1}"#,
            br#"{"GHD_CACHE_DISK":2}"#,
            br#"{"GHD_CACHE_TTLS":[]}"#,
        ] {
            assert!(parse(bad).is_err());
        }
    }
}
