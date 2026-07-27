//! Navigation: go-to-definition, find-references, and document symbols.
//!
//! Two tiers. While the file parses, `aura_lang::resolve` answers exactly — it
//! knows scopes, so `x` in a lambda and `x` at the top level stay distinct, and
//! uses inside `#{...}` count. While the file does not parse (i.e. most of the
//! time you are typing) the older lexical pass keeps answering approximately,
//! which is better than answering nothing.

use std::path::{Path, PathBuf};

use aura_lang::lexer::{Lexer, TokenKind};
use aura_lang::parser::Parser;
use aura_lang::resolve::{self, Resolution};
use aura_lang::span::Span;
use aura_lang::vfs::{FileResolver, ImportSpec, LocalFsResolver, ModuleId};
use lsp_types::{DocumentSymbol, Range, SymbolKind};

use crate::diagnostics::LineIndex;

/// Resolution for `text`, or `None` if it does not currently parse.
pub fn resolution(text: &str) -> Option<Resolution> {
    let toks = Lexer::new(text, 0).tokenize().ok()?;
    let module = Parser::new(toks).parse_module().ok()?;
    Some(resolve::resolve(text, &module))
}

fn to_range(index: &LineIndex, text: &str, s: Span) -> Range {
    Range {
        start: index.position(text, s.start as usize),
        end: index.position(text, s.end as usize),
    }
}

/// The identifier name whose token span covers `offset`.
fn ident_at<'a>(toks: &[aura_lang::lexer::Token<'a>], offset: u32) -> Option<&'a str> {
    toks.iter().find_map(|t| match t.kind {
        TokenKind::Ident(n) if t.span.start <= offset && offset <= t.span.end => Some(n),
        _ => None,
    })
}

pub fn definition_range(text: &str, line: u32, character: u32) -> Option<Range> {
    let toks = Lexer::new(text, 0).tokenize().ok()?;
    let index = LineIndex::new(text);
    let offset = index.offset(text, line, character) as u32;

    // Exact answer while the file parses.
    if let Some(r) = resolution(text) {
        if let Some(b) = r.binding_at(offset) {
            return Some(to_range(&index, text, r.bindings[b].decl));
        }
        // A parsing file with no binding here (a method name, a field key) has no
        // definition — do not fall through and guess.
        return None;
    }

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

/// If the cursor is on a file import — its path string, its alias, or any use
/// of that alias (`lib.Endpoint`) — return the imported file's relative path so
/// the caller can open it. Registry imports (`org/pkg@v1`) are not resolved.
pub fn import_target_path(text: &str, line: u32, character: u32) -> Option<String> {
    let toks = Lexer::new(text, 0).tokenize().ok()?;
    let index = LineIndex::new(text);
    let offset = index.offset(text, line, character) as u32;

    // File imports: (alias, path, path token index, alias token index).
    let mut imports: Vec<(&str, &str, usize, usize)> = Vec::new();
    for i in 0..toks.len() {
        if !matches!(toks[i].kind, TokenKind::Import) {
            continue;
        }
        if let (Some(TokenKind::Str(path)), Some(t2), Some(t3)) = (
            toks.get(i + 1).map(|t| &t.kind),
            toks.get(i + 2),
            toks.get(i + 3),
        ) {
            if matches!(t2.kind, TokenKind::As) {
                if let TokenKind::Ident(alias) = t3.kind {
                    imports.push((alias, path, i + 1, i + 3));
                }
            }
        }
    }
    let covers = |idx: usize| toks[idx].span.start <= offset && offset <= toks[idx].span.end;
    // On the import's own path or alias token.
    if let Some((_, path, ..)) = imports.iter().find(|(_, _, p, a)| covers(*p) || covers(*a)) {
        return Some(path.to_string());
    }
    // On a use of an import alias elsewhere (`lib.Endpoint`).
    let name = ident_at(&toks, offset)?;
    imports
        .iter()
        .find(|(alias, ..)| *alias == name)
        .map(|(_, path, ..)| path.to_string())
}

/// If the cursor is on a registry import (`github/owner/repo@v1.2`) — its path
/// token or its alias, including a use like `pkg.thing` — return the cached module
/// file the import resolves to.
///
/// Version selection is delegated to `LocalFsResolver`, the same resolver the
/// interpreter uses: `@v1.2` picking `1.2.0.aura` out of the cache is a range
/// match, and a second implementation of that would be free to disagree with the
/// one that actually loads the module.
pub fn registry_target_path(
    text: &str,
    line: u32,
    character: u32,
    registry_dir: &Path,
) -> Option<PathBuf> {
    let toks = Lexer::new(text, 0).tokenize().ok()?;
    let index = LineIndex::new(text);
    let offset = index.offset(text, line, character) as u32;

    // Registry imports as (alias, path, version, path token index, alias token index).
    let mut imports: Vec<(&str, &str, &str, usize, usize)> = Vec::new();
    for i in 0..toks.len() {
        if !matches!(toks[i].kind, TokenKind::Import) {
            continue;
        }
        if let (Some(TokenKind::ImportPath { path, version }), Some(TokenKind::Ident(alias))) = (
            toks.get(i + 1).map(|t| &t.kind),
            toks.get(i + 3).map(|t| &t.kind),
        ) {
            imports.push((alias, path, version, i + 1, i + 3));
        }
    }
    let covers = |idx: usize| toks[idx].span.start <= offset && offset <= toks[idx].span.end;
    let (_, path, version, ..) = imports
        .iter()
        .find(|(_, _, _, p, a)| covers(*p) || covers(*a))
        .or_else(|| {
            // On a use of the alias elsewhere (`rust.step`).
            let name = ident_at(&toks, offset)?;
            imports.iter().find(|(alias, ..)| *alias == name)
        })?;

    let resolver = LocalFsResolver {
        root: PathBuf::from("."),
        registry_dir: registry_dir.to_path_buf(),
    };
    let id = resolver
        .resolve(&ImportSpec::Registry { path, version }, None)
        .ok()?;
    match id {
        // Mirrors `LocalFsResolver::load`'s cache layout.
        ModuleId::Registry { path, version } => {
            Some(registry_dir.join(path).join(format!("{version}.aura")))
        }
        _ => None,
    }
}

/// The import path bound to `alias`, if any (file imports only).
fn import_path_for<'a>(toks: &[aura_lang::lexer::Token<'a>], alias: &str) -> Option<&'a str> {
    for i in 0..toks.len() {
        if matches!(toks[i].kind, TokenKind::Import) {
            if let (Some(TokenKind::Str(path)), Some(a)) = (
                toks.get(i + 1).map(|t| &t.kind),
                toks.get(i + 3).map(|t| &t.kind),
            ) {
                if matches!(a, TokenKind::Ident(n) if *n == alias) {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// If the cursor is on `alias.Member` where `alias` is a file import, return the
/// module's relative path and the member name, so the caller can open the module
/// and jump to that declaration.
pub fn imported_member(text: &str, line: u32, character: u32) -> Option<(String, String)> {
    let toks = Lexer::new(text, 0).tokenize().ok()?;
    let index = LineIndex::new(text);
    let offset = index.offset(text, line, character) as u32;
    let cur = toks.iter().position(|t| {
        matches!(t.kind, TokenKind::Ident(_)) && t.span.start <= offset && offset <= t.span.end
    })?;
    let TokenKind::Ident(member) = toks[cur].kind else {
        return None;
    };
    // Preceded by `.`, and before the `.` an import alias.
    if cur < 2 || !matches!(toks[cur - 1].kind, TokenKind::Dot) {
        return None;
    }
    let TokenKind::Ident(alias) = toks[cur - 2].kind else {
        return None;
    };
    let path = import_path_for(&toks, alias)?;
    Some((path.to_string(), member.to_string()))
}

/// The range of a `def`/`type` declaration named `name` in a module's text
/// (used to jump into an imported file).
pub fn declaration_range_in(text: &str, name: &str) -> Option<Range> {
    let toks = Lexer::new(text, 0).tokenize().ok()?;
    let index = LineIndex::new(text);
    for (i, t) in toks.iter().enumerate() {
        if matches!(t.kind, TokenKind::Ident(n) if n == name)
            && matches!(
                i.checked_sub(1).map(|p| &toks[p].kind),
                Some(TokenKind::Def | TokenKind::Type | TokenKind::Enum)
            )
        {
            return Some(Range {
                start: index.position(text, t.span.start as usize),
                end: index.position(text, t.span.end as usize),
            });
        }
    }
    None
}

/// Every occurrence (declaration + uses) of the binding under the cursor. Scope
/// precise while the file parses; a same-name lexical sweep otherwise.
pub fn reference_ranges(text: &str, line: u32, character: u32) -> Vec<Range> {
    let Ok(toks) = Lexer::new(text, 0).tokenize() else {
        return Vec::new();
    };
    let index = LineIndex::new(text);
    let offset = index.offset(text, line, character) as u32;

    if let Some(r) = resolution(text) {
        return match r.binding_at(offset) {
            Some(b) => r
                .occurrences(b)
                .into_iter()
                .map(|s| to_range(&index, text, s))
                .collect(),
            None => Vec::new(),
        };
    }

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

/// A flat outline of the declarations in a file: `def`, `type`, `enum`,
/// `domain`, and import aliases.
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
            (TokenKind::Ident(n), Some(TokenKind::Enum)) => (*n, SymbolKind::ENUM),
            (TokenKind::Ident(n), Some(TokenKind::As)) => (*n, SymbolKind::MODULE),
            // `domain "name"` — the label follows the keyword.
            (TokenKind::Str(n), Some(TokenKind::Domain)) => (*n, SymbolKind::NAMESPACE),
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
    fn import_target_resolves_path_alias_and_uses() {
        let src = "import \"lib.aura\" as lib\nx: lib.foo\n";
        assert_eq!(import_target_path(src, 0, 10).as_deref(), Some("lib.aura")); // on path
        assert_eq!(import_target_path(src, 0, 21).as_deref(), Some("lib.aura")); // on alias
        assert_eq!(import_target_path(src, 1, 3).as_deref(), Some("lib.aura")); // on a use
        assert_eq!(import_target_path("x = 1\n", 0, 0), None);
    }

    #[test]
    fn imported_member_and_module_declaration() {
        let src = "import \"lib.aura\" as lib\nx: new lib.Endpoint\n";
        let (path, member) = imported_member(src, 1, 12).unwrap();
        assert_eq!((path.as_str(), member.as_str()), ("lib.aura", "Endpoint"));
        // and locating that declaration inside a module's text
        let m = "pub type Endpoint\n  host: String\nend\npub def make()\n  x: 1\nend\n";
        assert_eq!(declaration_range_in(m, "Endpoint").unwrap().start.line, 0);
        assert_eq!(declaration_range_in(m, "make").unwrap().start.line, 3);
        assert!(declaration_range_in(m, "nope").is_none());
    }

    #[test]
    fn references_finds_all_occurrences() {
        // `x` appears on lines 0, 1, 2
        let refs = reference_ranges("x = 1\ny: x\nz: x + 1\n", 0, 0);
        let lines: Vec<u32> = refs.iter().map(|r| r.start.line).collect();
        assert_eq!(lines, vec![0, 1, 2]);
    }

    #[test]
    fn references_are_scope_precise_while_the_file_parses() {
        // Two unrelated `x`: a top-level binding and a `def` parameter. Before
        // scope precision this returned all four occurrences for either cursor.
        let src = "x = 1
def f(x)
  y: x
end
z: x
";
        let lines = |line, ch| -> Vec<u32> {
            reference_ranges(src, line, ch)
                .iter()
                .map(|r| r.start.line)
                .collect()
        };
        assert_eq!(lines(0, 0), vec![0, 4], "top-level x: its decl and `z: x`");
        assert_eq!(lines(1, 6), vec![1, 2], "the parameter and `y: x`");
    }

    #[test]
    fn references_include_interpolated_uses() {
        let src = "name = \"api\"
id: \"#{name}-1\"
";
        let lines: Vec<u32> = reference_ranges(src, 0, 0)
            .iter()
            .map(|r| r.start.line)
            .collect();
        assert_eq!(lines, vec![0, 1], "the use inside #{{...}} counts");
    }

    #[test]
    fn definition_of_a_shadowed_name_picks_the_inner_binding() {
        let src = "p = 1
domain \"d\"
  shadow p = 2
  a: p
end
";
        // Cursor on `p` in `a: p` (line 3) -> the `shadow` on line 2, not line 0.
        assert_eq!(def_line(src, 3, 5), Some(2));
    }

    #[test]
    fn references_fall_back_to_a_lexical_sweep_when_the_file_does_not_parse() {
        // Mid-edit: the `def` has no `end`, so nothing parses.
        let src = "x = 1
def f(
y: x
";
        assert!(resolution(src).is_none(), "must not parse");
        assert!(
            !reference_ranges(src, 0, 0).is_empty(),
            "the lexical tier still answers"
        );
    }

    #[test]
    fn registry_import_resolves_to_the_cached_module_file() {
        // The repository's own registry cache: `@v1.2` must select `1.2.0.aura`,
        // the same range match the interpreter performs.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/registry");
        let src = "import github/actions/rust-cache@v1.2 as rust\nx: rust.name\n";
        let expect = dir.join("github/actions/rust-cache/1.2.0.aura");
        assert!(expect.exists(), "fixture missing: {}", expect.display());

        for (line, ch, what) in [
            (0, 20, "the import path"),
            (0, 42, "the alias"),
            (1, 4, "a use"),
        ] {
            let got = registry_target_path(src, line, ch, &dir)
                .unwrap_or_else(|| panic!("no target on {what}"));
            assert_eq!(got, expect, "on {what}");
        }
        // A version with no cached match resolves to nothing.
        let miss = "import github/actions/rust-cache@v9.9 as rust\n";
        assert_eq!(registry_target_path(miss, 0, 20, &dir), None);
        // A file import is not a registry import.
        assert_eq!(
            registry_target_path("import \"lib.aura\" as lib\n", 0, 10, &dir),
            None
        );
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
