//! Everything that ships the language must call it by the same number.
//!
//! `crates/aura-wasm` is its own workspace, so `cargo` never checks it against
//! the main one, and it had drifted: the crate said `0.1.0` while the language
//! it embeds said `0.1.1`, and its lockfile pinned `aura-lang 0.1.0` — a version
//! that was no longer what the sibling directory contained.
//!
//! That matters more here than in most projects. The published `0.1.1` and the
//! repository's `0.1.1` were not the same language: the published build had none
//! of D29–D32 and none of the format-bridge fixes, so `cargo install aura-lang`
//! produced behaviour that disagreed with the documentation built from `main`.
//! One number for two behaviours is precisely what a configuration language
//! exists to prevent, and the fact that it happened inside our own repository is
//! the argument for checking it mechanically.
//!
//! The VS Code extension is deliberately **not** in this list. It launches
//! whatever `aura-lsp` is on the path and embeds nothing, so its version is its
//! own; moving it in lockstep would announce a change it did not have. The plan
//! is to bring it onto the same number once, at the 1.0 release, and it can join
//! this list then — not before, or the first thing the check enforces is a
//! version that means nothing.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// The first `version = "…"` at the start of a line, which in a manifest is the
/// package's own — dependency versions are written inline inside braces.
fn declared_version(rel: &str) -> String {
    read(rel)
        .lines()
        .find_map(|l| l.strip_prefix("version = \""))
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("no version in {rel}"))
        .to_string()
}

/// The version a lockfile pins for `name`.
fn locked_version(rel: &str, name: &str) -> String {
    let text = read(rel);
    let needle = format!("name = \"{name}\"");
    let at = text
        .find(&needle)
        .unwrap_or_else(|| panic!("{name} is not in {rel}"));
    text[at..]
        .lines()
        .find_map(|l| l.strip_prefix("version = \""))
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("no version after {name} in {rel}"))
        .to_string()
}

#[test]
fn every_shipped_artefact_carries_the_same_version() {
    // The compiled-in constant is the one users see in `aura --version` and in
    // the header of `llms.txt`, so it is the reference rather than a file.
    let ours = env!("CARGO_PKG_VERSION");

    let checks: &[(&str, String)] = &[
        ("the workspace", declared_version("Cargo.toml")),
        (
            "the wasm crate",
            declared_version("crates/aura-wasm/Cargo.toml"),
        ),
        (
            "aura-lang in Cargo.lock",
            locked_version("Cargo.lock", "aura-lang"),
        ),
        (
            "aura-lsp in Cargo.lock",
            locked_version("Cargo.lock", "aura-lsp"),
        ),
        (
            "aura-lang in the wasm lockfile",
            locked_version("crates/aura-wasm/Cargo.lock", "aura-lang"),
        ),
        (
            "aura-wasm in the wasm lockfile",
            locked_version("crates/aura-wasm/Cargo.lock", "aura-wasm"),
        ),
    ];

    let wrong: Vec<String> = checks
        .iter()
        .filter(|(_, v)| v != ours)
        .map(|(what, v)| format!("{what} says {v}"))
        .collect();

    assert!(
        wrong.is_empty(),
        "the language is {ours}, but {}. \
         The wasm crate is a separate workspace, so `cargo` will not do this for you.",
        wrong.join("; ")
    );
}

/// The release workflow cuts its notes from the changelog section named after
/// the tag, and refuses to publish without one. Finding that out from a failed
/// release is finding it out too late.
#[test]
fn the_changelog_has_a_section_for_this_version() {
    let version = env!("CARGO_PKG_VERSION");
    let changelog = read("CHANGELOG.md");
    assert!(
        changelog.contains(&format!("## [{version}]")),
        "CHANGELOG.md has no `## [{version}]` section; the release workflow \
         verifies this and will refuse the tag."
    );
}
