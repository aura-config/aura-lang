//! Go-to-definition: resolve the identifier under the cursor to its declaration.
//!
//! Lexical (token-based) so it works even while the file does not fully parse.
//! It resolves variables (`name = …`), functions (`def name`), schemas
//! (`type Name`) and import aliases (`… as name`). Scope-precise resolution and
//! parameters are a later refinement.

use aura_core::lexer::{Lexer, TokenKind};
use lsp_types::Range;

use crate::diagnostics::LineIndex;

pub fn definition_range(text: &str, line: u32, character: u32) -> Option<Range> {
    let toks = Lexer::new(text, 0).tokenize().ok()?;
    let index = LineIndex::new(text);
    let offset = index.offset(text, line, character) as u32;

    // The identifier under the cursor.
    let name = toks.iter().find_map(|t| match t.kind {
        TokenKind::Ident(n) if t.span.start <= offset && offset <= t.span.end => Some(n),
        _ => None,
    })?;

    // All declarations of that name, by token index.
    let mut defs: Vec<usize> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        let TokenKind::Ident(n) = t.kind else {
            continue;
        };
        if n != name {
            continue;
        }
        let prev = i.checked_sub(1).map(|p| &toks[p].kind);
        let next = toks.get(i + 1).map(|t| &t.kind);
        let is_var = matches!(next, Some(TokenKind::Assign))
            && matches!(prev, None | Some(TokenKind::Newline | TokenKind::Shadow));
        let is_decl = matches!(prev, Some(TokenKind::Def | TokenKind::Type | TokenKind::As));
        if is_var || is_decl {
            defs.push(i);
        }
    }
    if defs.is_empty() {
        return None;
    }
    // The nearest declaration at or before the cursor (a forward reference falls
    // back to the first).
    let chosen = defs
        .iter()
        .copied()
        .rfind(|&i| toks[i].span.start <= offset)
        .unwrap_or(defs[0]);
    let sp = toks[chosen].span;
    Some(Range {
        start: index.position(text, sp.start as usize),
        end: index.position(text, sp.end as usize),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def_line(text: &str, line: u32, ch: u32) -> Option<u32> {
        definition_range(text, line, ch).map(|r| r.start.line)
    }

    #[test]
    fn jumps_from_use_to_variable_definition() {
        // cursor on `x` in `y: x` (line 1) -> definition on line 0
        assert_eq!(def_line("x = 1\ny: x\n", 1, 3), Some(0));
    }

    #[test]
    fn jumps_to_def_and_type_and_alias() {
        let src = "import \"lib.aura\" as lib\ntype Meta\n  n: Int\nend\ndef make(a)\n  x: a\nend\nm: make(1)\nt: lib\n";
        // `make` use on the last-but-one line -> its `def` (line 4)
        assert_eq!(def_line(src, 7, 4), Some(4));
        // `lib` alias use -> the import (line 0)
        assert_eq!(def_line(src, 8, 3), Some(0));
    }

    #[test]
    fn method_name_has_no_definition() {
        // `.map` is a stdlib method, not a local declaration
        assert_eq!(def_line("y: xs.map\n", 0, 7), None);
    }
}
