use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Token<'a> {
    pub kind: TokenKind<'a>,
    pub span: Span,
}

/// SPEC §2.1. Zero-copy invariant: not a single `String` - only slices of the source.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind<'a> {
    // Literals
    Ident(&'a str),
    Int(i64),
    Float(f64),
    /// Content without quotes; escape sequences are raw (lazily unescaped during eval).
    Str(&'a str),
    InterpStr(Vec<StrPart<'a>>),
    /// github/actions/rust-cache@v1.2 → path="github/actions/rust-cache", version="v1.2"
    ImportPath {
        path: &'a str,
        version: &'a str,
    },
    True,
    False,
    Null,
    // Keywords
    Import,
    As,
    Type,
    Def,
    End,
    Domain,
    Component,
    New,
    Assert,
    Shadow,
    Pub,
    Cond,
    Else,
    // Delimiters
    Newline,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Colon,
    Comma,
    Dot,
    Assign,
    Arrow,
    Question,
    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    EqEq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    And,
    Or,
    Not,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StrPart<'a> {
    Lit(&'a str),
    /// A raw expression slice from `#{...}`; parsed by Phase 2.
    Interp(&'a str),
}

/// Every reserved keyword. The single source of truth for tooling (the LSP,
/// grammars): a keyword added to `keyword()` below must be listed here too, and
/// a test enforces that. `text` is intentionally absent — it is a contextual
/// block-string opener (D16), not a reserved word.
pub const KEYWORDS: &[&str] = &[
    "import",
    "as",
    "type",
    "def",
    "end",
    "domain",
    "component",
    "new",
    "assert",
    "shadow",
    "pub",
    "cond",
    "else",
    "true",
    "false",
    "null",
];

impl TokenKind<'_> {
    pub(crate) fn keyword(ident: &str) -> Option<TokenKind<'static>> {
        Some(match ident {
            "import" => TokenKind::Import,
            "as" => TokenKind::As,
            "type" => TokenKind::Type,
            "def" => TokenKind::Def,
            "end" => TokenKind::End,
            "domain" => TokenKind::Domain,
            "component" => TokenKind::Component,
            "new" => TokenKind::New,
            "assert" => TokenKind::Assert,
            "shadow" => TokenKind::Shadow,
            "pub" => TokenKind::Pub,
            "cond" => TokenKind::Cond,
            "else" => TokenKind::Else,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "null" => TokenKind::Null,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_const_matches_the_recognizer() {
        // Every listed keyword is recognized, and a non-keyword is not: the const
        // and `keyword()` cannot drift apart without this test going red.
        for kw in KEYWORDS {
            assert!(
                TokenKind::keyword(kw).is_some(),
                "`{kw}` is in KEYWORDS but not recognized by keyword()"
            );
        }
        assert!(TokenKind::keyword("text").is_none(), "text is contextual");
        assert!(TokenKind::keyword("nope").is_none());
    }
}
