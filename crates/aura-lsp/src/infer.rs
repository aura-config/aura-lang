//! Best-effort receiver-type inference for type-aware method completion.
//!
//! Given `receiver.<cursor>`, infer the type of `receiver` so completion can
//! offer only that type's methods. Token-based and deliberately partial: it
//! covers literals, list literals, builtin/method-call chains (using the
//! manifest's return types), and simple variables (`x = <expr>`). Anything it
//! cannot resolve returns `None`, and completion falls back to all methods.

use std::collections::HashMap;

use aura_lang::lexer::{Lexer, Token, TokenKind};

use crate::stdlib::Stdlib;

/// Type name (`"String"`, `"List"`, …) of the receiver being completed at
/// `offset` (which must be just after `receiver.<prefix>`), or `None`.
pub fn receiver_type(stdlib: &Stdlib, text: &str, offset: usize) -> Option<String> {
    let toks = Lexer::new(text, 0).tokenize().ok()?;
    // Byte offset of the `.` before the identifier prefix being typed.
    let bytes = text.as_bytes();
    let mut i = offset.min(bytes.len());
    while i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
        i -= 1;
    }
    if i == 0 || bytes[i - 1] != b'.' {
        return None;
    }
    let dot = (i - 1) as u32;
    let dot_idx = toks.iter().position(|t| {
        matches!(t.kind, TokenKind::Dot) && t.span.start <= dot && dot < t.span.end
    })?;

    let returns = name_returns(stdlib);
    infer(&toks, dot_idx, &returns, 0)
}

/// Method-name -> return type, but only for names whose return type is the same
/// across every receiver (so a bare name resolves unambiguously). `len` -> Int,
/// `to_json` -> String, etc.; ambiguous or `Any` names are omitted.
fn name_returns(stdlib: &Stdlib) -> HashMap<String, String> {
    let mut seen: HashMap<String, Option<String>> = HashMap::new();
    for e in stdlib.methods() {
        let entry = seen
            .entry(e.name.clone())
            .or_insert(Some(e.returns.clone()));
        if entry.as_deref() != Some(e.returns.as_str()) {
            *entry = None; // conflicting return types across receivers
        }
    }
    seen.into_iter()
        .filter_map(|(k, v)| v.filter(|r| r != "Any").map(|r| (k, r)))
        .collect()
}

/// Infer the type of the postfix expression whose last token is `toks[end-1]`.
fn infer(
    toks: &[Token],
    end: usize,
    returns: &HashMap<String, String>,
    depth: u32,
) -> Option<String> {
    if depth > 8 || end == 0 {
        return None;
    }
    let p = end - 1;
    let ty = |s: &str| Some(s.to_string());
    match &toks[p].kind {
        TokenKind::Str(_) | TokenKind::InterpStr(_) => ty("String"),
        TokenKind::Int(_) => ty("Int"),
        TokenKind::Float(_) => ty("Float"),
        TokenKind::True | TokenKind::False => ty("Bool"),
        TokenKind::RBracket => {
            let open = match_back(toks, p, TokenKind::LBracket, TokenKind::RBracket)?;
            // `[ … ]` after a value is an index (element type unknown); otherwise
            // it is a list literal.
            let is_index = open
                .checked_sub(1)
                .is_some_and(|b| value_ends(&toks[b].kind));
            if is_index {
                None
            } else {
                ty("List")
            }
        }
        TokenKind::RParen => {
            let open = match_back(toks, p, TokenKind::LParen, TokenKind::RParen)?;
            let callee = open.checked_sub(1)?;
            let TokenKind::Ident(name) = &toks[callee].kind else {
                return None; // a parenthesized expression, not a call
            };
            let is_method = callee >= 1 && matches!(toks[callee - 1].kind, TokenKind::Dot);
            if is_method {
                returns.get(*name).cloned()
            } else {
                match *name {
                    "range" => ty("List"),
                    "env" | "read_file" => ty("String"),
                    _ => None, // a user `def` (returns an object of unknown shape)
                }
            }
        }
        TokenKind::Ident(name) => {
            if p >= 1 && matches!(toks[p - 1].kind, TokenKind::Dot) {
                None // a field access: fields are untyped
            } else {
                var_type(toks, name, returns, depth)
            }
        }
        _ => None,
    }
}

/// Type of a variable by finding its `name = <expr>` and inferring the RHS.
fn var_type(
    toks: &[Token],
    name: &str,
    returns: &HashMap<String, String>,
    depth: u32,
) -> Option<String> {
    for (i, t) in toks.iter().enumerate() {
        let is_assign = matches!(t.kind, TokenKind::Ident(n) if n == name)
            && matches!(toks.get(i + 1).map(|t| &t.kind), Some(TokenKind::Assign))
            && matches!(
                i.checked_sub(1).map(|p| &toks[p].kind),
                None | Some(TokenKind::Newline | TokenKind::Shadow)
            );
        if !is_assign {
            continue;
        }
        // RHS runs to the end of the logical line (the next top-level Newline).
        let start = i + 2;
        let mut end = start;
        while end < toks.len() && !matches!(toks[end].kind, TokenKind::Newline | TokenKind::Eof) {
            end += 1;
        }
        return infer(toks, end, returns, depth + 1);
    }
    None
}

/// Index backward from a closing delimiter to its matching opener.
fn match_back(
    toks: &[Token],
    close: usize,
    open_k: TokenKind,
    close_k: TokenKind,
) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = close;
    loop {
        let k = &toks[i].kind;
        if std::mem::discriminant(k) == std::mem::discriminant(&close_k) {
            depth += 1;
        } else if std::mem::discriminant(k) == std::mem::discriminant(&open_k) {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i = i.checked_sub(1)?;
    }
}

/// Whether a token can end a value (so a following `[` is an index, not a list).
fn value_ends(k: &TokenKind) -> bool {
    matches!(
        k,
        TokenKind::Ident(_)
            | TokenKind::RParen
            | TokenKind::RBracket
            | TokenKind::Str(_)
            | TokenKind::InterpStr(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(text: &str) -> Option<String> {
        // cursor is at end of text, which must be `receiver.`
        receiver_type(&Stdlib::load(), text, text.len())
    }

    #[test]
    fn literal_receivers() {
        assert_eq!(rt("x: \"hi\"."), Some("String".into()));
        // `42.` alone is an incomplete-float lex error; a method prefix lexes fine.
        assert_eq!(rt("x: 42.abs"), Some("Int".into()));
        assert_eq!(rt("x: [1, 2]."), Some("List".into()));
        assert_eq!(rt("x: true."), Some("Bool".into()));
    }

    #[test]
    fn call_chains_via_manifest_returns() {
        assert_eq!(rt("x: \"a\".upper()."), Some("String".into())); // upper -> String
        assert_eq!(rt("x: xs.sort()."), Some("List".into())); // sort -> List
        assert_eq!(rt("x: range(3)."), Some("List".into())); // builtin
        assert_eq!(rt("x: env(\"A\", \"b\")."), Some("String".into()));
    }

    #[test]
    fn variable_resolution() {
        assert_eq!(rt("s = \"x\"\ny: s."), Some("String".into()));
        assert_eq!(rt("xs = [1]\ny: xs."), Some("List".into()));
    }

    #[test]
    fn unknown_falls_back_to_none() {
        assert_eq!(rt("y: unknownvar."), None);
        assert_eq!(rt("y: xs.first()."), None); // returns Any
    }
}
