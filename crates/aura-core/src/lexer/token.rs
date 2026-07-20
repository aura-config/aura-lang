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
