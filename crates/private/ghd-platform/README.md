# `ghd-platform`

The host-platform facade. One `cfg_select!` selector, a cfg-free
facility surface, and every line of native code confined to
`src/platforms/**`. This is the pattern soldr (`crates/soldr-platform`),
zccache, and kernal-api all use — copied here at a deliberately smaller
scope (one real capability instead of a full process/fs/ipc SDK) to
prove the shape without a second native-SDK dependency in the
template. See [zackees/ci.yml#6](https://github.com/zackees/ci.yml/issues/6)
comment 1, "Reference: the platforms/ facade pattern".

`publish = false`: see `crates/private/README.md` for the rules every
crate under `crates/private/` follows. This crate is also the **only**
place `ci.toml`'s `[allow] platform-selector` /
`platform-code` name as legal locations for host `cfg` / native code.

## Layout

```
src/
├── lib.rs                 # the ONE host selector (cfg_select!)
├── platform.rs             # cfg-free facade root
├── platform/
│   └── host.rs              # facade: os_name(), exe_suffix()
└── platforms/                # the ONLY native code in this crate
    ├── windows/{mod,host}.rs
    ├── linux/{mod,host}.rs
    └── macos/{mod,host}.rs
```

## The rule

- **One selector.** `lib.rs`'s `cfg_select!` has exactly three arms —
  `target_os = "windows"`, `"linux"`, `"macos"` — no `_` fallback and
  no `unix` arm. An OS this crate doesn't implement fails to compile
  here, loudly, instead of silently degrading.
- **One alias per consumer.** Every other crate that needs the OS adds
  exactly one line: `pub(crate) use ghd_platform as platform;`
  (see `ghd-core/src/lib.rs`). No crate outside this one names
  `platforms::windows`/`platforms::linux`/`platforms::macos` directly.
- **The facade owns the types.** `platform.rs` and `platform/**`
  contain **no `cfg` of their own** — they only re-export whatever
  `lib.rs` selected as `crate::platform_imp`. Add a capability by
  adding a function to `platform/host.rs` that forwards to
  `platform_imp::host::*`, then implementing it once per OS under
  `platforms/*/host.rs`.
- **Duplicate across trees rather than add a selector.** If Windows
  and Linux need different logic for the same fact, that logic lives
  once in each `platforms/<os>/host.rs` — never as a second `cfg` gate
  layered on top of the one in `lib.rs`.
- **Runtime tests, never `cfg`.** The unit tests in `lib.rs` call
  `host::os_name()` / `host::exe_suffix()` and assert on what the
  *running* binary reports (cross-checked against
  `std::env::consts::EXE_SUFFIX`), never on what `cfg!(windows)` claims
  at compile time. A test that branches on `cfg` to decide what to
  expect isn't testing anything.

## Why `cfg_select!` and not plain `#[cfg(...)]`

`cfg_select!` (`std::cfg_select!`) picks exactly one arm's *items* at
parse time — no arm's code exists in the others' compilation at all,
and there's no way to accidentally fall through to a shared default.
Verified against this workspace's pinned stable toolchain
(`rust-toolchain.toml` channel `1.95.0`) with a throwaway crate before
adopting it here; it compiled cleanly, so this crate uses it rather
than three separate `#[cfg(target_os = "...")] mod platforms_x;`
declarations gated by hand.

## The one capability

`host::os_name() -> &'static str` (`"windows"` / `"linux"` / `"macos"`)
and `host::exe_suffix() -> &'static str` (`".exe"` on Windows, empty
elsewhere). Small on purpose — see the crate-level doc comment in
`lib.rs`. Grow this facade the same way soldr grew
`soldr-platform`: add a namespace (`process`, `fs`, ...) to
`platform.rs`'s doc list, a file under `platform/`, and one function
per OS under each `platforms/*/`.

## Testing

`ghd-platform:lib` in `ci.toml`'s `[rust.tests].binaries`.
`[lib] doctest = false`. Run with `soldr cargo test --workspace
--locked`.
