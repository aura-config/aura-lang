// Diagnostic is deliberately passed by value: errors are the cold path, boxing
// would complicate every signature for a micro-optimization.
#![allow(clippy::result_large_err)]
// Environment uses RefCell per the D9 design (single-threaded frame construction,
// see eval/env.rs); Arc was chosen for future domain-level parallelism.
#![allow(clippy::arc_with_non_send_sync)]

/// The Aura LANGUAGE version (grammar and semantics per SPEC.md); does not match the crate version.
pub const LANGUAGE_VERSION: &str = "1.3";

pub mod analysis;
pub mod error;
pub mod eval;
pub mod facade;
pub mod fmt;
pub mod lexer;
pub mod parser;
pub mod serialize;
pub mod source;
pub mod span;
pub mod vfs;
