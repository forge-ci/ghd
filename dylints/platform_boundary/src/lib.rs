#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_errors;
extern crate rustc_span;

use rustc_errors::DiagDecorator;
use rustc_lint::{EarlyContext, EarlyLintPass, LintContext};
use rustc_span::{FileName, RemapPathScopeComponents, Span};
use std::collections::HashSet;

#[derive(Default)]
struct PlatformBoundary {
    scanned_files: HashSet<String>,
}

dylint_linting::impl_pre_expansion_lint! {
    /// ### What it does
    ///
    /// Enforces this template's `platforms/` facade boundary
    /// (`ci.yml#6` §"Reference: the platforms/ facade pattern"; `ci.toml`'s
    /// `[allow] platform-selector` / `platform-code`): the only source file
    /// allowed to choose the host platform is
    /// `crates/private/ghd-platform/src/lib.rs` (the `cfg_select!`
    /// selector). Every other production Rust source in the workspace denies
    /// host-platform `#[cfg]`, `#[cfg_attr]`, `cfg!(...)`, and `cfg_select!`.
    /// Direct references to native platform paths (`std::os::windows`,
    /// `std::os::unix`, `libc::`, `windows_sys::`) are additionally allowed
    /// under `crates/private/ghd-platform/src/platforms/**` (the
    /// concrete per-OS implementation trees `ci.toml` names as
    /// `platform-code`). Direct references to the concrete module aliases
    /// (`platform_imp`, `platform_windows`, `platform_linux`,
    /// `platform_macos`) are allowed anywhere inside the
    /// `ghd-platform` crate (its cfg-free facade re-exports them) but
    /// denied in every other crate — downstream code reaches only
    /// `crate::platform::host` (see that crate's `platform.rs`).
    ///
    /// Inspection happens pre-expansion, so cfg'd-away code cannot hide.
    /// Integration tests, examples, and benches are scanned as part of the
    /// same boundary.
    ///
    /// ### Second-order effect: one Linux Dylint job is sufficient for the
    /// selector part of the boundary
    ///
    /// Because this lint runs pre-expansion and denies host `#[cfg]` outside
    /// `ghd-platform/src/lib.rs`, host-specific *selection* logic can
    /// only exist in that one file, regardless of which target Dylint's
    /// driver happens to be checking. The per-target `cfg(windows)` /
    /// `cfg(target_os = "macos")` *type-checking* still needs the per-target
    /// Dylint passes this workspace's `dylint` lane runs from Linux
    /// (`soldr cargo dylint -- --target T`, prebuilt `rust-std` only, no
    /// per-target driver build) — this lint alone does not prove those
    /// branches compile, only that no *new* host-selection site opened
    /// outside the boundary.
    ///
    /// ### Attribution
    ///
    /// Pattern, scan logic, and comment-and-string-aware source scanning
    /// adapted from soldr's `dylints/ban_platform_cfg_outside_boundary`
    /// (zackees/soldr issues #2493, #2498), itself following
    /// kernal-api's `docs/platform-boundary.md`. Adapted for this
    /// workspace's single-selector-file shape (soldr's boundary instead
    /// allows three concrete `platform_{win,linux,macos}` trees to select
    /// with their own `cfg`; this template's `platforms/**` trees select
    /// via `lib.rs`'s `#[path = ...]` module declarations and carry no
    /// `cfg` of their own — see `zackees/ci.yml#6` round 2).
    pub PLATFORM_BOUNDARY,
    Deny,
    "keep host-platform selection inside ghd-platform's cfg_select boundary",
    PlatformBoundary::default()
}

const SELECTORS: [&str; 10] = [
    "windows",
    "unix",
    "target_os",
    "target_family",
    "target_arch",
    "target_abi",
    "target_env",
    "target_vendor",
    "target_endian",
    "target_pointer_width",
];

/// The ONE file allowed to select the host. `ci.toml`'s
/// `[allow] platform-selector`.
const SELECTOR_FILE: &str = "crates/private/ghd-platform/src/lib.rs";

/// Native platform paths (`std::os::*`, `libc::`, `windows_sys::`) are
/// additionally allowed under this prefix. `ci.toml`'s `[allow]
/// platform-code` (Rust half; its `src/ghd/platforms/**`
/// entry is Python and outside this lint's scope).
const PLATFORM_CODE_PREFIX: &str = "crates/private/ghd-platform/src/platforms/";

/// Concrete-tree module aliases may be referenced anywhere inside this
/// crate (the cfg-free facade in `platform.rs` re-exports them); denied in
/// every other crate.
const PLATFORM_CRATE_DIR: &str = "crates/private/ghd-platform/";

impl EarlyLintPass for PlatformBoundary {
    fn check_item(&mut self, cx: &EarlyContext<'_>, item: &rustc_ast::ast::Item) {
        let current_file = source_filename(cx, item.span);
        if !in_scope(&current_file) || !self.scanned_files.insert(current_file.clone()) {
            return;
        }
        // Scan the physical source once, rather than only the active AST.
        // This is what makes cfg-elided items and crate-level inner attrs
        // visible to the boundary.
        let source = std::fs::read_to_string(&current_file)
            .or_else(|_| cx.sess().source_map().span_to_snippet(item.span));
        let Ok(source) = source else { return };
        let relative = relative_path(&current_file);
        let is_selector_file = relative.as_deref() == Some(SELECTOR_FILE);
        let is_platform_code =
            relative.as_deref().is_some_and(|r| r.starts_with(PLATFORM_CODE_PREFIX));
        let is_inside_platform_crate =
            relative.as_deref().is_some_and(|r| r.starts_with(PLATFORM_CRATE_DIR));

        if !is_selector_file {
            for invocation in platform_cfg_invocations(&source) {
                emit(cx, item.span, format!("host cfg `{invocation}`"));
            }
            if contains_cfg_select(&source) {
                emit(cx, item.span, "host selection macro `cfg_select!`".to_owned());
            }
        }
        if !is_selector_file && !is_platform_code {
            for reference in native_platform_references(&source) {
                emit(cx, item.span, format!("direct native-platform reference `{reference}`"));
            }
        }
        if !is_inside_platform_crate {
            for reference in concrete_tree_references(&source) {
                emit(cx, item.span, format!("direct concrete-tree reference `{reference}`"));
            }
        }
    }
}

fn emit(cx: &EarlyContext<'_>, span: Span, detail: String) {
    cx.opt_span_lint(
        PLATFORM_BOUNDARY,
        Some(span),
        DiagDecorator(move |diag| {
            diag.primary_message(format!(
                "host-platform selection outside the ghd-platform boundary: {detail}; \
                 the only allowed selection site is \
                 crates/private/ghd-platform/src/lib.rs, and native platform paths are \
                 additionally allowed under \
                 crates/private/ghd-platform/src/platforms/**"
            ));
        }),
    );
}

/// Production sources the boundary applies to: every `crates/**.rs` file
/// (including unit-test modules inside them), plus every Dylint UI-test
/// fixture, minus the one fixture standing in for what the boundary
/// itself allows.
fn in_scope(filename: &str) -> bool {
    let normalized = filename.replace('\\', "/");
    if normalized.ends_with("ui/allowed_boundary.rs") {
        return false;
    }
    if normalized.starts_with("ui/") || normalized.contains("/ui/") {
        return true;
    }
    let Some(marker) = normalized.find("crates/") else {
        return false;
    };
    normalized[marker..].ends_with(".rs")
}

/// The workspace-relative path starting at `crates/`, or `None` for a file
/// outside the workspace tree (for example a Dylint UI-test fixture, which
/// is checked from an isolated temp directory with no `crates/` ancestor).
fn relative_path(filename: &str) -> Option<String> {
    let normalized = filename.replace('\\', "/");
    normalized.find("crates/").map(|marker| normalized[marker..].to_owned())
}

fn platform_cfg_invocations(source: &str) -> Vec<String> {
    let code = code_without_comments_or_strings(source);
    let compact: String = code.chars().filter(|character| !character.is_whitespace()).collect();
    let mut invocations = Vec::new();
    for start in ["#[cfg(", "#[cfg_attr(", "#![cfg(", "#![cfg_attr(", "cfg!("] {
        for (offset, _) in compact.match_indices(start) {
            let Some(clause) = balanced_invocation(&compact, offset) else {
                continue;
            };
            if SELECTORS.iter().any(|selector| clause.contains(selector)) {
                invocations.push(clause.trim_start_matches("#[").to_owned());
            }
        }
    }
    invocations
}

fn contains_cfg_select(source: &str) -> bool {
    code_without_comments_or_strings(source).contains("cfg_select!")
}

fn concrete_tree_references(source: &str) -> Vec<String> {
    let code = code_without_comments_or_strings(source);
    let mut references = Vec::new();
    for name in ["platform_imp", "platform_windows", "platform_linux", "platform_macos"] {
        // Word-boundary scan: the identifier must stand alone.
        for (offset, _) in code.match_indices(name) {
            let before = code[..offset].chars().next_back();
            let after = code[offset + name.len()..].chars().next();
            let standalone = |c: Option<char>| c.is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
            if standalone(before) && standalone(after) {
                references.push(name.to_owned());
            }
        }
    }
    references
}

fn native_platform_references(source: &str) -> Vec<String> {
    let code = code_without_comments_or_strings(source);
    ["std::os::windows", "std::os::unix", "std::os::linux", "std::os::macos", "windows_sys", "libc::"]
        .into_iter()
        .filter(|marker| code.contains(marker))
        .map(str::to_owned)
        .collect()
}

fn balanced_invocation(source: &str, offset: usize) -> Option<&str> {
    let mut depth = 0_u32;
    let mut saw_open = false;
    for (relative, character) in source[offset..].char_indices() {
        match character {
            '(' => {
                saw_open = true;
                depth += 1;
            }
            ')' if saw_open => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[offset..offset + relative + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

fn code_without_comments_or_strings(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = String::with_capacity(source.len());
    let mut index = 0;
    while index < bytes.len() {
        if let Some((prefix_len, hashes)) = raw_string_prefix(&bytes[index..]) {
            let start = index;
            index += prefix_len;
            while index < bytes.len() {
                let suffix = &bytes[index + 1..];
                if bytes[index] == b'"' && suffix.len() >= hashes && suffix[..hashes].iter().all(|byte| *byte == b'#')
                {
                    index += 1 + hashes;
                    break;
                }
                index += 1;
            }
            mask_range(&mut output, bytes, start, index);
        } else if bytes[index..].starts_with(b"//") {
            while index < bytes.len() && bytes[index] != b'\n' {
                output.push(' ');
                index += 1;
            }
        } else if bytes[index..].starts_with(b"/*") {
            let mut depth = 1_u32;
            output.push_str("  ");
            index += 2;
            while index < bytes.len() && depth > 0 {
                if bytes[index..].starts_with(b"/*") {
                    depth += 1;
                    output.push_str("  ");
                    index += 2;
                } else if bytes[index..].starts_with(b"*/") {
                    depth -= 1;
                    output.push_str("  ");
                    index += 2;
                } else {
                    output.push(if bytes[index] == b'\n' { '\n' } else { ' ' });
                    index += 1;
                }
            }
        } else if bytes[index] == b'"' {
            output.push(' ');
            index += 1;
            while index < bytes.len() {
                let byte = bytes[index];
                output.push(if byte == b'\n' { '\n' } else { ' ' });
                index += 1;
                if byte == b'\\' && index < bytes.len() {
                    output.push(' ');
                    index += 1;
                } else if byte == b'"' {
                    break;
                }
            }
        } else {
            output.push(bytes[index] as char);
            index += 1;
        }
    }
    output
}

fn raw_string_prefix(source: &[u8]) -> Option<(usize, usize)> {
    let mut index = usize::from(source.starts_with(b"br"));
    if source.get(index) != Some(&b'r') {
        return None;
    }
    index += 1;
    let hashes_start = index;
    while source.get(index) == Some(&b'#') {
        index += 1;
    }
    (source.get(index) == Some(&b'"')).then_some((index + 1, index - hashes_start))
}

fn mask_range(output: &mut String, source: &[u8], start: usize, end: usize) {
    for byte in &source[start..end] {
        output.push(if *byte == b'\n' { '\n' } else { ' ' });
    }
}

fn source_filename(cx: &EarlyContext<'_>, span: Span) -> String {
    match cx.sess().source_map().span_to_filename(span) {
        FileName::Real(real_filename) => real_filename
            .local_path()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                real_filename.path(RemapPathScopeComponents::DIAGNOSTICS).to_string_lossy().into_owned()
            }),
        filename => filename.display(RemapPathScopeComponents::DIAGNOSTICS).to_string(),
    }
}

#[test]
fn ui() {
    dylint_testing::ui_test(env!("CARGO_PKG_NAME"), "ui");
}

#[test]
fn source_detector_ignores_comments_and_strings() {
    assert!(platform_cfg_invocations(
        r####"fn neutral() {
            let _ = "#[cfg(windows)]";
            let _ = r###"cfg!(target_arch = "x86_64")"###;
            /* cfg!(unix) */
        }"####
    )
    .is_empty());
    assert_eq!(
        platform_cfg_invocations("fn selected() { if cfg!(windows) {} }"),
        vec!["cfg!(windows)"]
    );
    // feature cfgs are allowed: only the host selectors count
    assert!(platform_cfg_invocations("#[cfg(feature = \"tokio-console\")] fn f() {}").is_empty());
    // test-only cfg is still host selection when a host selector appears
    assert_eq!(
        platform_cfg_invocations("#[cfg(all(test, target_os = \"windows\"))] fn f() {}"),
        vec!["cfg(all(test,target_os=))"]
    );
}

#[test]
fn cfg_select_is_detected_outside_strings_and_comments() {
    assert!(contains_cfg_select("fn f() { let _ = \"cfg_select!\"; }") == false);
    assert!(contains_cfg_select("use std::cfg_select; cfg_select! { unix => {}, }"));
}

#[test]
fn concrete_references_are_word_boundary_matched() {
    assert_eq!(
        concrete_tree_references("crate::platform_imp::host::x(); platform_windows::y();"),
        vec!["platform_imp", "platform_windows"]
    );
    assert!(concrete_tree_references("platform_windows32; platform_windowsx;").is_empty());
}

#[test]
fn native_platform_references_are_detected_outside_strings() {
    assert_eq!(
        native_platform_references("use std::os::unix::fs::PermissionsExt; libc::getpid();"),
        vec!["std::os::unix", "libc::"]
    );
    assert!(native_platform_references("let text = \"windows_sys\";").is_empty());
}

#[test]
fn integration_tests_examples_and_benches_are_in_scope() {
    for path in [
        "crates/template/tests/host.rs",
        "crates/template/examples/host.rs",
        "crates/template/benches/host.rs",
    ] {
        assert!(in_scope(path), "{path}");
    }
}

#[test]
fn platform_code_and_selector_file_are_identified_correctly() {
    assert_eq!(
        relative_path("/runner/work/repo/crates/private/ghd-platform/src/lib.rs").as_deref(),
        Some(SELECTOR_FILE)
    );
    assert!(relative_path("/runner/work/repo/crates/private/ghd-platform/src/platforms/linux/host.rs")
        .as_deref()
        .unwrap()
        .starts_with(PLATFORM_CODE_PREFIX));
}
