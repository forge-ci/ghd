//! Request-local controls are distinct from retention identity and daemon defaults.
use ghd_core::request_policy::Policy;
kernal_api::task_local! { pub(crate) static CURRENT: Policy = POLICY_STORAGE; }
pub(crate) fn current() -> Policy {
    CURRENT.try_with(|p| *p).unwrap_or_default()
}
pub(crate) fn ttl(default: std::time::Duration) -> std::time::Duration {
    current().ttl(default)
}
