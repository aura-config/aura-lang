//! The Aura configuration language: a library, and the `aura` CLI behind the
//! default `cli` feature.
//!
//! # Which surface is supported
//!
//! [`facade`] is the API for embedding. It takes a path or a set of in-memory
//! buffers and returns `serde_json::Value` together with plain diagnostics, and
//! it is the part meant to stay stable.
//!
//! The layers beneath — [`lexer`], [`parser`], [`eval`], [`source`], [`resolve`]
//! — are public because tools need them: the language server, the browser
//! playground, and `examples/scripting.rs`, which calls a manifest's `pub def`
//! from Rust. They are useful and they are not frozen. While the project is
//! `0.x` they may change in a minor release, so prefer [`facade`] unless you
//! need something it does not expose.
//!
//! Embedding the library alone:
//!
//! ```toml
//! [dependencies]
//! aura-lang = { version = "0.1", default-features = false }
//! ```

// Diagnostic is deliberately passed by value: errors are the cold path, boxing
// would complicate every signature for a micro-optimization.
#![allow(clippy::result_large_err)]
// Environment uses RefCell per the D9 design (single-threaded frame construction,
// see eval/env.rs); Arc was chosen for future domain-level parallelism.
#![allow(clippy::arc_with_non_send_sync)]

/// The stdlib surface as an Aura manifest: every method's parameters, return
/// type and one-line description, grouped by receiver.
///
/// It describes the language rather than any one tool, so it lives here and is
/// read by everything that needs to talk about the standard library — the
/// language server's completion database, and `aura docs --agent`. Keeping one
/// copy is the point: a test checks it against the real method registry, and a
/// second copy would only be checked by nobody.
pub const STDLIB_MANIFEST: &str = include_str!("../stdlib.aura");

pub mod agentdocs;
pub mod analysis;
pub mod codegen;
pub mod error;
pub mod eval;
pub mod facade;
pub mod fmt;
pub mod lexer;
pub mod parser;
pub mod resolve;
pub mod serialize;
pub mod source;
pub mod span;
pub mod vfs;
