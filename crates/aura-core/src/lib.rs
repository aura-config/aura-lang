// Diagnostic намеренно передаётся по значению: ошибки — холодный путь, боксинг
// усложнил бы каждую сигнатуру ради микрооптимизации.
#![allow(clippy::result_large_err)]
// Environment использует RefCell по дизайну D9 (однопоточное построение фреймов,
// см. eval/env.rs); Arc выбран под будущую параллельность доменов.
#![allow(clippy::arc_with_non_send_sync)]

/// Версия ЯЗЫКА Aura (грамматика и семантика по SPEC.md); не совпадает с версией крейта.
pub const LANGUAGE_VERSION: &str = "1.2";

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
