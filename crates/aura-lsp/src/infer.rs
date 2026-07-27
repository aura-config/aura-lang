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

// ---- D18: enum member completion ----

/// Enum declarations in a file: name -> members.
fn enum_decls<'a>(toks: &[Token<'a>]) -> HashMap<&'a str, Vec<&'a str>> {
    let mut out = HashMap::new();
    let mut i = 0;
    while i < toks.len() {
        if !matches!(toks[i].kind, TokenKind::Enum) {
            i += 1;
            continue;
        }
        let Some(TokenKind::Ident(name)) = toks.get(i + 1).map(|t| &t.kind) else {
            i += 1;
            continue;
        };
        let mut members = Vec::new();
        let mut j = i + 2;
        while j < toks.len() && !matches!(toks[j].kind, TokenKind::End) {
            if let TokenKind::Str(m) = &toks[j].kind {
                members.push(*m);
            }
            j += 1;
        }
        out.insert(*name, members);
        i = j;
    }
    out
}

/// Schema declarations: schema name -> (field name -> declared type name).
fn schema_field_types<'a>(toks: &[Token<'a>]) -> HashMap<&'a str, HashMap<&'a str, &'a str>> {
    let mut out = HashMap::new();
    let mut i = 0;
    while i < toks.len() {
        if !matches!(toks[i].kind, TokenKind::Type) {
            i += 1;
            continue;
        }
        let Some(TokenKind::Ident(sname)) = toks.get(i + 1).map(|t| &t.kind) else {
            i += 1;
            continue;
        };
        let mut fields = HashMap::new();
        let mut j = i + 2;
        while j < toks.len() && !matches!(toks[j].kind, TokenKind::End) {
            if let (TokenKind::Ident(f), Some(TokenKind::Colon), Some(TokenKind::Ident(ty))) = (
                &toks[j].kind,
                toks.get(j + 1).map(|t| &t.kind),
                toks.get(j + 2).map(|t| &t.kind),
            ) {
                fields.insert(*f, *ty);
            }
            j += 1;
        }
        out.insert(*sname, fields);
        i = j;
    }
    out
}

/// The schema being instantiated by the `new` block that encloses `at`, plus the
/// field key whose value the cursor sits in.
/// The `new` block enclosing `at`: `(module alias, schema, field)`. The alias is
/// `None` for `new Schema` and `Some("lib")` for `new lib.Schema`, which decides
/// whether the field's type is looked up in this file or in the imported module.
#[allow(clippy::type_complexity)]
fn enclosing_new_field<'a>(
    toks: &[Token<'a>],
    at: usize,
) -> Option<(Option<&'a str>, &'a str, &'a str)> {
    // The token index just before the cursor.
    let cur = toks.iter().rposition(|t| (t.span.start as usize) < at)?;

    // The field key: nearest `Ident Colon` pair at or before the cursor.
    let mut k = cur;
    let field = loop {
        if matches!(toks[k].kind, TokenKind::Colon) && k > 0 {
            if let TokenKind::Ident(f) = &toks[k - 1].kind {
                break *f;
            }
        }
        k = k.checked_sub(1)?;
    };

    // Walk back to the `new` that opens this block, skipping nested blocks.
    let mut depth = 0usize;
    let mut i = k;
    loop {
        match &toks[i].kind {
            TokenKind::End => depth += 1,
            TokenKind::New if depth == 0 => {
                // `new Schema` or `new alias.Schema`
                return match (
                    toks.get(i + 1).map(|t| &t.kind),
                    toks.get(i + 2).map(|t| &t.kind),
                    toks.get(i + 3).map(|t| &t.kind),
                ) {
                    (
                        Some(TokenKind::Ident(alias)),
                        Some(TokenKind::Dot),
                        Some(TokenKind::Ident(s)),
                    ) => Some((Some(*alias), *s, field)),
                    (Some(TokenKind::Ident(s)), _, _) => Some((None, *s, field)),
                    _ => None,
                };
            }
            TokenKind::New
            | TokenKind::Domain
            | TokenKind::Def
            | TokenKind::Type
            | TokenKind::Enum
            | TokenKind::Cond => depth = depth.saturating_sub(1),
            _ => {}
        }
        i = i.checked_sub(1)?;
    }
}

/// Members of the enum expected at `offset`: the cursor is in the value position
/// of a `new Schema` field whose declared type is a declared enum.
///
/// For `new alias.Schema` the schema and its enum live in the imported module, so
/// `load_module` is called with the import's relative path to get that file's text.
/// Returning `None` from it (an unresolvable or registry import) simply means no
/// members are offered.
pub fn expected_enum_members(
    text: &str,
    offset: usize,
    load_module: impl Fn(&str) -> Option<String>,
) -> Option<Vec<String>> {
    let toks = Lexer::new(text, 0).tokenize().ok()?;
    let (alias, schema, field) = enclosing_new_field(&toks, offset)?;

    let Some(alias) = alias else {
        return members_of_field(text, schema, field);
    };
    // `new lib.Schema` — resolve inside the module bound to `lib`.
    let path = import_path_for(&toks, alias)?;
    let module = load_module(path)?;
    members_of_field(&module, schema, field)
}

/// The enum members of `schema.field`, both declared in `text`.
fn members_of_field(text: &str, schema: &str, field: &str) -> Option<Vec<String>> {
    let toks = Lexer::new(text, 0).tokenize().ok()?;
    let ty = *schema_field_types(&toks).get(schema)?.get(field)?;
    let members = enum_decls(&toks).get(ty)?.clone();
    Some(members.into_iter().map(str::to_string).collect())
}

/// The relative path of the file import bound to `alias`. Registry imports
/// (`org/pkg@v1`) lex as `ImportPath`, not `Str`, so they yield `None`.
fn import_path_for<'a>(toks: &[Token<'a>], alias: &str) -> Option<&'a str> {
    for i in 0..toks.len() {
        if !matches!(toks[i].kind, TokenKind::Import) {
            continue;
        }
        if let (Some(TokenKind::Str(path)), Some(TokenKind::Ident(a))) = (
            toks.get(i + 1).map(|t| &t.kind),
            toks.get(i + 3).map(|t| &t.kind),
        ) {
            if *a == alias {
                return Some(path);
            }
        }
    }
    None
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

    const ENUM_SRC: &str = concat!(
        "enum Tier
  \"frontend\"
  \"backend\"
  \"cache\"
end
",
        "type Service
  name: String
  tier: Tier
end
",
        "svc: new Service
  name: \"api\"
  tier: 
",
    );

    #[test]
    fn enum_members_offered_for_an_enum_typed_field() {
        // cursor at the end, i.e. in the value position of `tier:`
        let m = expected_enum_members(ENUM_SRC, ENUM_SRC.len(), |_| None).expect("members");
        assert_eq!(m, vec!["frontend", "backend", "cache"]);
    }

    #[test]
    fn no_enum_members_for_a_plain_field() {
        // `name: String` is not an enum, so no member list
        let upto = ENUM_SRC.find("tier: ").unwrap() - 1;
        assert_eq!(expected_enum_members(ENUM_SRC, upto, |_| None), None);
    }

    /// A `pub enum` field of an imported schema still offers its members: the
    /// declarations live in the module file, reached through the import path.
    #[test]
    fn enum_members_offered_for_an_imported_schema() {
        const LIB: &str = concat!(
            "pub enum Scheme
  \"https\"
  \"http\"
end
",
            "pub type Endpoint
  host: String
  scheme: Scheme
end
",
        );
        let src = concat!(
            "import \"lib.aura\" as lib
",
            "e: new lib.Endpoint
  host: \"h\"
  scheme: ",
        );
        let load = |path: &str| (path == "lib.aura").then(|| LIB.to_string());
        assert_eq!(
            expected_enum_members(src, src.len(), load),
            Some(vec!["https".to_string(), "http".to_string()])
        );
        // A plain field of the same imported schema offers nothing.
        let upto = src.find("host: ").unwrap() + 6;
        assert_eq!(expected_enum_members(src, upto, load), None);
    }

    /// The same thing against the files actually shipped in the repository, so the
    /// wiring is proven on real declarations rather than on hand-written fixtures.
    #[test]
    fn imported_enum_members_on_the_real_showcase_files() {
        const LIB: &str = include_str!("../../../examples/showcase/lib.aura");
        // `lib.Endpoint.scheme` is typed by `pub enum Scheme` in lib.aura.
        let src = "import \"lib.aura\" as lib
e: new lib.Endpoint
  host: \"h\"
  scheme: ";
        assert_eq!(
            expected_enum_members(src, src.len(), |p| (p == "lib.aura")
                .then(|| LIB.to_string())),
            Some(vec!["https".to_string(), "http".to_string()])
        );
        // `host: String` in the same schema is not an enum.
        let upto = src.find("host: ").unwrap() + 6;
        assert_eq!(
            expected_enum_members(src, upto, |p| (p == "lib.aura").then(|| LIB.to_string())),
            None
        );
    }

    #[test]
    fn unresolvable_and_registry_imports_offer_nothing() {
        let src = concat!(
            "import \"lib.aura\" as lib
",
            "e: new lib.Endpoint
  scheme: ",
        );
        // The module cannot be read (unsaved, missing, outside the workspace).
        assert_eq!(expected_enum_members(src, src.len(), |_| None), None);
        // A registry import has no file path to load at all.
        let reg = concat!(
            "import acme/net@v1.0.0 as net
",
            "e: new net.Endpoint
  scheme: ",
        );
        assert_eq!(
            expected_enum_members(reg, reg.len(), |_| panic!("must not be loaded")),
            None
        );
    }

    #[test]
    fn no_enum_members_outside_a_new_block() {
        let src = "enum Tier
  \"a\"
end
x: 
";
        assert_eq!(expected_enum_members(src, src.len(), |_| None), None);
    }

    #[test]
    fn unknown_falls_back_to_none() {
        assert_eq!(rt("y: unknownvar."), None);
        assert_eq!(rt("y: xs.first()."), None); // returns Any
    }
}
