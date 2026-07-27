//! Signature help: parameter hints while typing a call.
//!
//! Resolves the innermost unclosed `name(` before the cursor, then looks the
//! callee up in the dogfooded stdlib manifest (methods and builtins) or among the
//! file's own `def`s. The active parameter is the number of top-level commas
//! typed so far.

use aura_lang::lexer::{Lexer, Token, TokenKind};

use crate::stdlib::Stdlib;

pub struct Signature {
    /// e.g. `replace(from, to)`
    pub label: String,
    pub params: Vec<String>,
    /// Index of the parameter being typed.
    pub active: u32,
    pub doc: Option<String>,
}

/// Parameters of a `def name(a, b)` declared in this file.
fn local_def_params<'a>(toks: &[Token<'a>], name: &str) -> Option<Vec<&'a str>> {
    for i in 0..toks.len() {
        if !matches!(toks[i].kind, TokenKind::Def) {
            continue;
        }
        if !matches!(toks.get(i + 1).map(|t| &t.kind), Some(TokenKind::Ident(n)) if *n == name) {
            continue;
        }
        // `def name ( p , p )`
        let mut params = Vec::new();
        let mut j = i + 3; // skip name and `(`
        while j < toks.len() && !matches!(toks[j].kind, TokenKind::RParen) {
            if let TokenKind::Ident(p) = &toks[j].kind {
                params.push(*p);
            }
            j += 1;
        }
        return Some(params);
    }
    None
}

/// The signature of the call surrounding `offset`, if any.
pub fn signature_at(stdlib: &Stdlib, text: &str, offset: usize) -> Option<Signature> {
    let toks = Lexer::new(text, 0).tokenize().ok()?;
    // Tokens strictly before the cursor.
    let cur = toks
        .iter()
        .rposition(|t| (t.span.start as usize) < offset)?;

    // Innermost unclosed `(` at or before the cursor, counting commas after it.
    let mut depth = 0i32;
    let mut commas = 0u32;
    let mut i = cur;
    let open = loop {
        match toks[i].kind {
            TokenKind::RParen => depth += 1,
            TokenKind::LParen => {
                if depth == 0 {
                    break i;
                }
                depth -= 1;
            }
            TokenKind::Comma if depth == 0 => commas += 1,
            // A newline outside brackets ends the expression: no call here.
            TokenKind::Newline if depth == 0 => return None,
            _ => {}
        }
        i = i.checked_sub(1)?;
    };

    let callee = open.checked_sub(1)?;
    let TokenKind::Ident(name) = &toks[callee].kind else {
        return None; // a parenthesized expression, or a lambda's parameter list
    };
    let is_method = callee >= 1 && matches!(toks[callee - 1].kind, TokenKind::Dot);

    // Prefer a same-file `def`; otherwise the stdlib manifest.
    if !is_method {
        if let Some(params) = local_def_params(&toks, name) {
            let params: Vec<String> = params.into_iter().map(str::to_string).collect();
            return Some(Signature {
                label: format!("{name}({})", params.join(", ")),
                params,
                active: commas,
                doc: None,
            });
        }
    }
    let entry = stdlib
        .entries
        .iter()
        .find(|e| e.name == *name && (is_method) == (e.receiver != "builtins"))?;
    Some(Signature {
        label: entry.signature(),
        params: entry.params.clone(),
        active: commas,
        doc: Some(entry.doc.clone()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(text: &str) -> Option<Signature> {
        signature_at(&Stdlib::load(), text, text.len())
    }

    #[test]
    fn stdlib_method_signature_and_active_parameter() {
        let s = sig("x: \"a-b\".replace(").expect("signature");
        assert_eq!(s.label, "replace(from, to) -> String");
        assert_eq!(s.params, vec!["from", "to"]);
        assert_eq!(s.active, 0);

        // after the first comma the second parameter is active
        let s = sig("x: \"a-b\".replace(\"-\", ").expect("signature");
        assert_eq!(s.active, 1);
    }

    #[test]
    fn builtin_signature() {
        let s = sig("x: env(").expect("signature");
        assert_eq!(s.params, vec!["name", "default"]);
        assert!(s.doc.unwrap_or_default().contains("Environment"));
    }

    #[test]
    fn local_def_signature_wins_over_the_manifest() {
        let s = sig("def build(a, b)\n  v: 1\nend\nx: build(").expect("signature");
        assert_eq!(s.label, "build(a, b)");
        assert_eq!(s.params, vec!["a", "b"]);
    }

    #[test]
    fn no_signature_outside_a_call() {
        assert!(sig("x: 1 + 2\n").is_none());
        assert!(sig("x: [1, 2]\n").is_none());
    }
}
