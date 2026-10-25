//! Linux host facts: OS name and executable suffix.
//!
//! Real, host-specific answers — not a stub. `os_name` is a fixed literal
//! (there is no "detect the OS" step: the fact that this file compiled at
//! all already proves `target_os = "linux"`); `exe_suffix` is empty, which
//! is Linux's actual convention, not a placeholder.

pub(crate) fn os_name() -> &'static str {
    "linux"
}

pub(crate) fn exe_suffix() -> &'static str {
    ""
}
