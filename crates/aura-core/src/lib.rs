// Diagnostic намеренно передаётся по значению: ошибки — холодный путь, боксинг
// усложнил бы каждую сигнатуру ради микрооптимизации.
#![allow(clippy::result_large_err)]
// Environment использует RefCell по дизайну D9 (однопоточное построение фреймов,
// см. eval/env.rs); Arc выбран под будущую параллельность доменов.
#![allow(clippy::arc_with_non_send_sync)]

pub mod analysis;
pub mod error;
pub mod eval;
pub mod lexer;
pub mod parser;
pub mod serialize;
pub mod source;
pub mod span;
pub mod vfs;
