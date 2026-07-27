// Diagnostic is deliberately passed by value: errors are the cold path, boxing
// would complicate every signature for a micro-optimization.
#![allow(clippy::result_large_err)]
// Environment uses RefCell per the D9 design (single-threaded frame construction,
// see eval/env.rs); Arc was chosen for future domain-level parallelism.
#![allow(clippy::arc_with_non_send_sync)]

pub mod analysis;
pub mod codegen;
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
