//! `aura fmt`: канонизация отступов (SPEC §7.2).
//!
//! Форматтер строчно-ориентированный: текст строк (включая комментарии и
//! внутристрочное выравнивание) сохраняется, нормализуются только отступы
//! (2 пробела на уровень), хвостовые пробелы и пустые строки (максимум одна
//! подряд, без пустых в начале/конце файла).
//!
//! Глубина вложенности считается по токенам: `domain`/`component`/`def`/`type`/
//! `new`/`->`/`[`/`(` и `key:` в конце строки открывают уровень; `end`/`]`/`)` —
//! закрывают. Инвариант безопасности: поток токенов не меняется (см. тесты).

use crate::error::Diagnostic;
use crate::lexer::{Lexer, TokenKind};

const INDENT: &str = "  ";

#[derive(Default, Clone, Copy)]
struct LineInfo {
    /// Суммарное изменение глубины после строки.
    delta: i32,
    /// Минимум префиксной суммы внутри строки: ведущие `end`/`]` дают дедент самой строке.
    min_prefix: i32,
    /// Последний значимый токен — `:` (открывает объектный блок со следующей строки).
    ends_with_colon: bool,
    /// Строка обрывается посреди выражения (`,` `=` `.` `?` или бинарный оператор
    /// в конце) — следующая строка получает отступ продолжения (+1).
    continues: bool,
}

pub fn format_source(src: &str) -> Result<String, Diagnostic> {
    let tokens = Lexer::new(src, 0).tokenize()?;

    // Начала строк (байтовые смещения) для маппинга span → номер строки
    let mut line_starts = vec![0usize];
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    let line_of = |off: u32| line_starts.partition_point(|s| *s <= off as usize) - 1;

    let mut infos = vec![LineInfo::default(); line_starts.len()];
    for t in &tokens {
        let d: i32 = match t.kind {
            TokenKind::Domain
            | TokenKind::Component
            | TokenKind::Def
            | TokenKind::Type
            | TokenKind::New
            | TokenKind::Arrow
            | TokenKind::LBracket
            | TokenKind::LParen => 1,
            TokenKind::End | TokenKind::RBracket | TokenKind::RParen => -1,
            TokenKind::Newline | TokenKind::Eof => continue,
            _ => 0,
        };
        let info = &mut infos[line_of(t.span.start)];
        info.delta += d;
        info.min_prefix = info.min_prefix.min(info.delta);
        info.ends_with_colon = matches!(t.kind, TokenKind::Colon);
        info.continues = matches!(
            t.kind,
            TokenKind::Comma
                | TokenKind::Assign
                | TokenKind::Dot
                | TokenKind::Question
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
        );
    }

    let mut out = String::with_capacity(src.len());
    let mut depth: i32 = 0;
    let mut pending_blank = false;
    let mut wrote_any = false;
    let mut prev_continues = false;
    for (idx, line) in src.lines().enumerate() {
        let text = line.trim();
        if text.is_empty() {
            // максимум одна пустая строка, и не в начале файла
            pending_blank = wrote_any;
            continue;
        }
        if pending_blank {
            out.push('\n');
            pending_blank = false;
        }
        let info = infos[idx];
        let level = (depth + info.min_prefix.min(0) + i32::from(prev_continues)).max(0);
        for _ in 0..level {
            out.push_str(INDENT);
        }
        out.push_str(text);
        out.push('\n');
        wrote_any = true;
        depth += info.delta + i32::from(info.ends_with_colon);
        prev_continues = info.continues;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = include_str!("../../tests/fixtures/production_deploy.aura");

    fn kinds(src: &str) -> Vec<TokenKind<'_>> {
        Lexer::new(src, 0)
            .tokenize()
            .expect("lex ok")
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn normalizes_messy_indentation() {
        let messy =
            "domain \"d\"\n      x = 1\n  security:\n        tls: true\n  end\n\n\n\ny: x\nend\n";
        let want = "domain \"d\"\n  x = 1\n  security:\n    tls: true\n  end\n\n  y: x\nend\n";
        assert_eq!(format_source(messy).unwrap(), want);
    }

    #[test]
    fn preserves_comments_and_inline_alignment() {
        let src = "# заголовок\nbase_port        = 8000  # выравнивание сохраняется\n";
        assert_eq!(format_source(src).unwrap(), src);
    }

    /// Инвариант безопасности: форматирование не меняет поток токенов.
    #[test]
    fn token_stream_is_preserved() {
        let formatted = format_source(MANIFEST).unwrap();
        assert_eq!(
            kinds(MANIFEST),
            kinds(&formatted),
            "fmt must not change semantics"
        );
    }

    /// Идемпотентность: повторный прогон ничего не меняет.
    #[test]
    fn is_idempotent() {
        let once = format_source(MANIFEST).unwrap();
        let twice = format_source(&once).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn continuation_lines_get_extra_indent() {
        let src = "assert !(a && b),\n\"message\"\nx = 1 +\n2\n";
        let want = "assert !(a && b),\n  \"message\"\nx = 1 +\n  2\n";
        assert_eq!(format_source(src).unwrap(), want);
    }

    #[test]
    fn multiline_lists_and_lambdas() {
        let messy =
            "xs = [\n\"a\"\n\"b\"\n]\napps: xs.map (n, i) ->\ncomponent n\nimage: n\nend\nend\n";
        let want = "xs = [\n  \"a\"\n  \"b\"\n]\napps: xs.map (n, i) ->\n  component n\n    image: n\n  end\nend\n";
        assert_eq!(format_source(messy).unwrap(), want);
    }
}
