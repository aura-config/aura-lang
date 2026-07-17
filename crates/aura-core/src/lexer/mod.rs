pub mod token;

use crate::error::Diagnostic;
use crate::span::{SourceId, Span};
pub use token::{StrPart, Token, TokenKind};

pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    source: SourceId,
    expect_import_path: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str, source: SourceId) -> Self {
        Lexer { src, pos: 0, source, expect_import_path: false }
    }

    /// Fail-fast на первой лексической ошибке (SPEC §2.6).
    pub fn tokenize(mut self) -> Result<Vec<Token<'a>>, Diagnostic> {
        let mut raw: Vec<Token<'a>> = Vec::new();
        loop {
            self.skip_trivia(&mut raw);
            let start = self.pos;
            let Some(c) = self.peek() else {
                raw.push(Token { kind: TokenKind::Eof, span: self.span(start) });
                break;
            };
            let kind = match c {
                b'"' => self.lex_string()?,
                b'0'..=b'9' => self.lex_number()?,
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                    if self.expect_import_path {
                        self.lex_import_path()?
                    } else {
                        self.lex_ident_or_keyword()
                    }
                }
                _ => self.lex_operator()?,
            };
            // Контекстный режим путей импорта действует ровно на один следующий токен (SPEC §2.4).
            self.expect_import_path = matches!(kind, TokenKind::Import);
            raw.push(Token { kind, span: self.span(start) });
        }
        Ok(normalize(raw))
    }

    fn span(&self, start: usize) -> Span {
        Span::new(self.source, start, self.pos)
    }

    fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    fn peek_at(&self, off: usize) -> Option<u8> {
        self.src.as_bytes().get(self.pos + off).copied()
    }

    fn bump(&mut self) {
        // Продвижение на целый UTF-8 символ; ASCII-путь — на байт.
        let step = self.src[self.pos..].chars().next().map_or(1, |c| c.len_utf8());
        self.pos += step;
    }

    /// Пробелы, `\r`, комментарии `#...`, схлопывание `\n+` в один Newline (SPEC §2.5, правило 1).
    fn skip_trivia(&mut self, out: &mut Vec<Token<'a>>) {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r') => self.pos += 1,
                Some(b'#') => {
                    while !matches!(self.peek(), None | Some(b'\n')) {
                        self.bump();
                    }
                }
                Some(b'\n') => {
                    let start = self.pos;
                    self.pos += 1;
                    if !matches!(out.last().map(|t| &t.kind), Some(TokenKind::Newline)) {
                        out.push(Token { kind: TokenKind::Newline, span: self.span(start) });
                    }
                }
                _ => break,
            }
        }
    }

    /// ДКА чисел (SPEC §2.2): Int(i64) | Float(f64); `12.foo` — откат, `12.` — E0101.
    fn lex_number(&mut self) -> Result<TokenKind<'a>, Diagnostic> {
        let start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        if self.peek() == Some(b'.') {
            match self.peek_at(1) {
                Some(b'0'..=b'9') => {
                    self.pos += 1;
                    while matches!(self.peek(), Some(b'0'..=b'9')) {
                        self.pos += 1;
                    }
                    let text = &self.src[start..self.pos];
                    return Ok(TokenKind::Float(text.parse().expect("DFA guarantees valid float")));
                }
                Some(b'a'..=b'z' | b'A'..=b'Z' | b'_') => {} // откат: Int, затем Dot, Ident
                _ => {
                    return Err(Diagnostic::error(
                        "E0101",
                        "digit expected after decimal point",
                        Span::new(self.source, start, self.pos + 1),
                        "incomplete float literal",
                    ))
                }
            }
        }
        let text = &self.src[start..self.pos];
        text.parse::<i64>().map(TokenKind::Int).map_err(|_| {
            Diagnostic::error("E0103", "integer literal overflows i64", self.span(start), "does not fit in i64")
        })
    }

    /// ДКА строк (SPEC §2.3): чистый срез без escape/interp; InterpStr при `#{...}`.
    fn lex_string(&mut self) -> Result<TokenKind<'a>, Diagnostic> {
        let quote_start = self.pos;
        self.pos += 1; // открывающая кавычка
        let content_start = self.pos;
        let mut parts: Vec<StrPart<'a>> = Vec::new();
        let mut lit_start = self.pos;
        loop {
            match self.peek() {
                None | Some(b'\n') => {
                    return Err(Diagnostic::error(
                        "E0102",
                        "unterminated string literal",
                        self.span(quote_start),
                        "string is not closed before end of line",
                    ))
                }
                Some(b'"') => {
                    let kind = if parts.is_empty() {
                        TokenKind::Str(&self.src[content_start..self.pos])
                    } else {
                        if lit_start < self.pos {
                            parts.push(StrPart::Lit(&self.src[lit_start..self.pos]));
                        }
                        TokenKind::InterpStr(parts)
                    };
                    self.pos += 1;
                    return Ok(kind);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    if matches!(self.peek(), None | Some(b'\n')) {
                        return Err(Diagnostic::error(
                            "E0102",
                            "unterminated string literal",
                            self.span(quote_start),
                            "escape at end of line",
                        ));
                    }
                    self.bump();
                }
                Some(b'#') if self.peek_at(1) == Some(b'{') => {
                    if lit_start < self.pos {
                        parts.push(StrPart::Lit(&self.src[lit_start..self.pos]));
                    }
                    self.pos += 2;
                    let expr_start = self.pos;
                    let mut depth = 1u32;
                    loop {
                        match self.peek() {
                            None | Some(b'\n') => {
                                return Err(Diagnostic::error(
                                    "E0106",
                                    "unterminated string interpolation",
                                    Span::new(self.source, expr_start - 2, self.pos),
                                    "missing closing '}'",
                                ))
                            }
                            Some(b'{') => depth += 1,
                            Some(b'}') => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        self.bump();
                    }
                    parts.push(StrPart::Interp(&self.src[expr_start..self.pos]));
                    self.pos += 1; // закрывающая '}'
                    lit_start = self.pos;
                }
                _ => self.bump(),
            }
        }
    }

    fn lex_ident_or_keyword(&mut self) -> TokenKind<'a> {
        let start = self.pos;
        while matches!(self.peek(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')) {
            self.pos += 1;
        }
        let text = &self.src[start..self.pos];
        TokenKind::keyword(text).unwrap_or(TokenKind::Ident(text))
    }

    /// SPEC §2.4 (D8): `path("/" path)* "@" "v" digits ("." digits){0,2}`; без версии — E0104.
    fn lex_import_path(&mut self) -> Result<TokenKind<'a>, Diagnostic> {
        let start = self.pos;
        while matches!(self.peek(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'/')) {
            self.pos += 1;
        }
        let path = &self.src[start..self.pos];
        // Одиночный идентификатор без слэшей мог бы быть переменной, но грамматика import
        // допускает только путь или строку — трактуем как путь и требуем версию.
        if self.peek() != Some(b'@') {
            return Err(Diagnostic::error(
                "E0104",
                "registry import requires a version",
                self.span(start),
                format!("add a version, e.g. `{path}@v1`"),
            ));
        }
        self.pos += 1;
        let ver_start = self.pos;
        let ok = self.peek() == Some(b'v') && {
            self.pos += 1;
            let mut groups = 0;
            loop {
                let digits_start = self.pos;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
                if self.pos == digits_start {
                    break false;
                }
                groups += 1;
                if groups == 3 || self.peek() != Some(b'.') {
                    break true;
                }
                self.pos += 1;
            }
        };
        if !ok {
            return Err(Diagnostic::error(
                "E0104",
                "malformed import version",
                Span::new(self.source, ver_start.saturating_sub(1), self.pos),
                "expected `@vX`, `@vX.Y` or `@vX.Y.Z`",
            ));
        }
        Ok(TokenKind::ImportPath { path, version: &self.src[ver_start..self.pos] })
    }

    fn lex_operator(&mut self) -> Result<TokenKind<'a>, Diagnostic> {
        let start = self.pos;
        let two = |k| Some((2usize, k));
        let pair = (self.peek().unwrap(), self.peek_at(1));
        let m = match pair {
            (b'-', Some(b'>')) => two(TokenKind::Arrow),
            (b'=', Some(b'=')) => two(TokenKind::EqEq),
            (b'!', Some(b'=')) => two(TokenKind::NotEq),
            (b'<', Some(b'=')) => two(TokenKind::LtEq),
            (b'>', Some(b'=')) => two(TokenKind::GtEq),
            (b'&', Some(b'&')) => two(TokenKind::And),
            (b'|', Some(b'|')) => two(TokenKind::Or),
            (c, _) => {
                let k = match c {
                    b'(' => TokenKind::LParen,
                    b')' => TokenKind::RParen,
                    b'[' => TokenKind::LBracket,
                    b']' => TokenKind::RBracket,
                    b':' => TokenKind::Colon,
                    b',' => TokenKind::Comma,
                    b'.' => TokenKind::Dot,
                    b'=' => TokenKind::Assign,
                    b'?' => TokenKind::Question,
                    b'+' => TokenKind::Plus,
                    b'-' => TokenKind::Minus,
                    b'*' => TokenKind::Star,
                    b'/' => TokenKind::Slash,
                    b'%' => TokenKind::Percent,
                    b'<' => TokenKind::Lt,
                    b'>' => TokenKind::Gt,
                    b'!' => TokenKind::Not,
                    _ => {
                        self.bump();
                        return Err(Diagnostic::error(
                            "E0105",
                            "unexpected character",
                            self.span(start),
                            "not a valid Aura token",
                        ));
                    }
                };
                Some((1, k))
            }
        };
        let (len, kind) = m.unwrap();
        self.pos += len;
        Ok(kind)
    }
}

/// Пост-фильтр нормализации `\n` (SPEC §2.5, правила 2–7) со стеком скобок.
fn normalize(raw: Vec<Token<'_>>) -> Vec<Token<'_>> {
    let mut out: Vec<Token<'_>> = Vec::with_capacity(raw.len());
    let mut stack: Vec<u8> = Vec::new();
    for i in 0..raw.len() {
        let t = &raw[i];
        if let TokenKind::Newline = t.kind {
            let prev = out.last().map(|t| &t.kind);
            let next = raw.get(i + 1).map(|t| &t.kind);
            let keep = match stack.last() {
                // Внутри (...) переносы подавляются полностью (правило 4).
                Some(b'(') => false,
                // Внутри [...] перенос — разделитель элементов, кроме краёв и после ',' (правила 5–6, D2).
                Some(b'[') => {
                    !matches!(prev, None | Some(TokenKind::LBracket | TokenKind::Comma))
                        && !matches!(next, None | Some(TokenKind::RBracket))
                }
                // Вне скобок: правила 2, 3, 7. `:` намеренно НЕ подавляет — перенос после
                // `key:` значим, он открывает объектный блок (SPEC §3.2).
                _ => {
                    let after = !matches!(
                        prev,
                        None | Some(
                            TokenKind::Assign
                                | TokenKind::Arrow
                                | TokenKind::Question
                                | TokenKind::Comma
                                | TokenKind::Dot
                                | TokenKind::Plus
                                | TokenKind::Minus
                                | TokenKind::Star
                                | TokenKind::Slash
                                | TokenKind::Percent
                                | TokenKind::EqEq
                                | TokenKind::NotEq
                                | TokenKind::Lt
                                | TokenKind::Gt
                                | TokenKind::LtEq
                                | TokenKind::GtEq
                                | TokenKind::And
                                | TokenKind::Or
                        )
                    );
                    let before = !matches!(next, None | Some(TokenKind::Dot | TokenKind::Eof));
                    after && before
                }
            };
            if keep {
                out.push(t.clone());
            }
            continue;
        }
        match t.kind {
            TokenKind::LParen => stack.push(b'('),
            TokenKind::LBracket => stack.push(b'['),
            TokenKind::RParen | TokenKind::RBracket => {
                stack.pop();
            }
            _ => {}
        }
        out.push(t.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind<'_>> {
        Lexer::new(src, 0).tokenize().expect("lex ok").into_iter().map(|t| t.kind).collect()
    }

    fn err(src: &str) -> &'static str {
        Lexer::new(src, 0).tokenize().expect_err("lex must fail").code
    }

    use TokenKind::*;

    #[test]
    fn list_newline_is_element_separator_d2() {
        // [a \n -b] — однозначно два элемента (SPEC D2)
        assert_eq!(
            kinds("[a\n-b\n]"),
            vec![LBracket, Ident("a"), Newline, Minus, Ident("b"), RBracket, Eof]
        );
    }

    #[test]
    fn newline_suppressed_after_binary_op_at_top_level() {
        assert_eq!(kinds("x = 1 +\n2"), vec![Ident("x"), Assign, Int(1), Plus, Int(2), Eof]);
    }

    #[test]
    fn newline_kept_after_colon_for_object_blocks() {
        // Перенос после `key:` значим — открывает объектный блок (SPEC §3.2)
        assert_eq!(kinds("security:\nx: 1\nend"), vec![
            Ident("security"), Colon, Newline, Ident("x"), Colon, Int(1), Newline, End, Eof
        ]);
    }

    #[test]
    fn newlines_inside_parens_fully_suppressed() {
        assert_eq!(kinds("f(\n1,\n2\n)"), vec![Ident("f"), LParen, Int(1), Comma, Int(2), RParen, Eof]);
    }

    #[test]
    fn newline_suppressed_before_dot_chain() {
        assert_eq!(kinds("a\n.b()"), vec![Ident("a"), Dot, Ident("b"), LParen, RParen, Eof]);
    }

    #[test]
    fn import_path_with_version_d8() {
        assert_eq!(
            kinds("import github/actions/rust-cache@v1.2 as rust"),
            vec![Import, ImportPath { path: "github/actions/rust-cache", version: "v1.2" }, As, Ident("rust"), Eof]
        );
    }

    #[test]
    fn import_path_without_version_is_e0104() {
        assert_eq!(err("import github/actions/rust-cache as rust"), "E0104");
        assert_eq!(err("import a/b@1.2 as x"), "E0104"); // без префикса 'v'
    }

    #[test]
    fn file_import_needs_no_version() {
        assert_eq!(
            kinds(r#"import "templates/x.aura" as d"#),
            vec![Import, Str("templates/x.aura"), As, Ident("d"), Eof]
        );
    }

    #[test]
    fn numbers_int_float_rollback_and_errors() {
        assert_eq!(kinds("12.5"), vec![Float(12.5), Eof]);
        assert_eq!(kinds("12.foo"), vec![Int(12), Dot, Ident("foo"), Eof]);
        assert_eq!(err("x = 12."), "E0101");
        assert_eq!(err("x = 99999999999999999999"), "E0103");
    }

    #[test]
    fn string_interpolation_parts() {
        assert_eq!(
            kinds(r#""company/#{name}:#{ver}""#),
            vec![
                InterpStr(vec![
                    StrPart::Lit("company/"),
                    StrPart::Interp("name"),
                    StrPart::Lit(":"),
                    StrPart::Interp("ver"),
                ]),
                Eof
            ]
        );
        assert_eq!(err("\"unterminated"), "E0102");
        assert_eq!(err("\"#{a + b\""), "E0106");
    }

    #[test]
    fn comments_and_blank_lines_collapse_to_one_newline() {
        assert_eq!(kinds("a = 1\n# comment\n\n\nb = 2"), vec![
            Ident("a"), Assign, Int(1), Newline, Ident("b"), Assign, Int(2), Eof
        ]);
    }

    #[test]
    fn keywords_v12() {
        assert_eq!(kinds("new assert shadow"), vec![New, Assert, Shadow, Eof]);
    }
}
