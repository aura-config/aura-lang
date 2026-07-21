//! Token-based navigation: go-to-definition, find-references, and document
//! symbols. Lexical so it works even while the file does not fully parse.
//! Scope-precise resolution and parameters are a later refinement.

use aura_core::lexer::{Lexer, TokenKind};
use lsp_types::{DocumentSymbol, Range, SymbolKind};

use crate::diagnostics::LineIndex;

/// The identifier name whose token span covers `offset`.
fn ident_at<'a>(toks: &[aura_core::lexer::Token<'a>], offset: u32) -> Option<&'a str> {
    toks.iter().find_map(|t| match t.kind {
        TokenKind::Ident(n) if t.span.start <= offset && offset <= t.span.end => Some(n),
        _ => None,
    })
}

pub fn definition_range(text: &str, line: u32, character: u32) -> Option<Range> {
    let toks = Lexer::new(text, 0).tokenize().ok()?;
    let index = LineIndex::new(text);
    let offset = index.offset(text, line, character) as u32;

    let name = ident_at(&toks, offset)?;

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

/// Every occurrence (declaration + uses) of the identifier under the cursor.
/// No scoping yet: same-named symbols in other scopes are included.
pub fn reference_ranges(text: &str, line: u32, character: u32) -> Vec<Range> {
    let Ok(toks) = Lexer::new(text, 0).tokenize() else {
        return Vec::new();
    };
    let index = LineIndex::new(text);
    let offset = index.offset(text, line, character) as u32;
    let Some(name) = ident_at(&toks, offset) else {
        return Vec::new();
    };
    toks.iter()
        .filter(|t| matches!(t.kind, TokenKind::Ident(n) if n == name))
        .map(|t| Range {
            start: index.position(text, t.span.start as usize),
            end: index.position(text, t.span.end as usize),
        })
        .collect()
}

/// A flat outline of the declarations in a file: `def`, `type`, `domain`,
/// `component`, and import aliases.
pub fn document_symbols(text: &str) -> Vec<DocumentSymbol> {
    let Ok(toks) = Lexer::new(text, 0).tokenize() else {
        return Vec::new();
    };
    let index = LineIndex::new(text);
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        let prev = i.checked_sub(1).map(|p| &toks[p].kind);
        let (name, kind) = match (&t.kind, prev) {
            (TokenKind::Ident(n), Some(TokenKind::Def)) => (*n, SymbolKind::FUNCTION),
            (TokenKind::Ident(n), Some(TokenKind::Type)) => (*n, SymbolKind::STRUCT),
            (TokenKind::Ident(n), Some(TokenKind::As)) => (*n, SymbolKind::MODULE),
            // domain "name" / component name — the label follows the keyword.
            (TokenKind::Str(n), Some(TokenKind::Domain | TokenKind::Component)) => {
                (*n, SymbolKind::NAMESPACE)
            }
            (TokenKind::Ident(n), Some(TokenKind::Component)) => (*n, SymbolKind::NAMESPACE),
            _ => continue,
        };
        let range = Range {
            start: index.position(text, t.span.start as usize),
            end: index.position(text, t.span.end as usize),
        };
        #[allow(deprecated)]
        out.push(DocumentSymbol {
            name: name.to_string(),
            detail: None,
            kind,
            tags: None,
            deprecated: None,
            range,
            selection_range: range,
            children: None,
        });
    }
    out
}

/// Declared names in a file (variables, `def`, `type`, import aliases), for
/// completion. Deduplicated, in declaration order.
pub fn local_names(text: &str) -> Vec<String> {
    let Ok(toks) = Lexer::new(text, 0).tokenize() else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        let TokenKind::Ident(n) = t.kind else {
            continue;
        };
        let prev = i.checked_sub(1).map(|p| &toks[p].kind);
        let next = toks.get(i + 1).map(|t| &t.kind);
        let is_var = matches!(next, Some(TokenKind::Assign))
            && matches!(prev, None | Some(TokenKind::Newline | TokenKind::Shadow));
        let is_decl = matches!(prev, Some(TokenKind::Def | TokenKind::Type | TokenKind::As));
        if (is_var || is_decl) && !names.iter().any(|s| s == n) {
            names.push(n.to_string());
        }
    }
    names
}

/// Whether the cursor at `offset` is completing a method (`receiver.<here>`):
/// the char before the identifier prefix being typed is a `.`.
pub fn is_method_context(text: &str, offset: usize) -> bool {
    let bytes = text.as_bytes();
    let mut i = offset.min(bytes.len());
    while i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
        i -= 1;
    }
    i > 0 && bytes[i - 1] == b'.'
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

    #[test]
    fn references_finds_all_occurrences() {
        // `x` appears on lines 0, 1, 2
        let refs = reference_ranges("x = 1\ny: x\nz: x + 1\n", 0, 0);
        let lines: Vec<u32> = refs.iter().map(|r| r.start.line).collect();
        assert_eq!(lines, vec![0, 1, 2]);
    }

    #[test]
    fn document_symbols_lists_declarations() {
        let src = "import \"lib.aura\" as lib\ntype Meta\n  n: Int\nend\ndef make(a)\n  x: a\nend\ndomain \"prod\"\nend\n";
        let syms = document_symbols(src);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["lib", "Meta", "make", "prod"]);
        assert_eq!(syms[1].kind, SymbolKind::STRUCT);
        assert_eq!(syms[2].kind, SymbolKind::FUNCTION);
    }
}
