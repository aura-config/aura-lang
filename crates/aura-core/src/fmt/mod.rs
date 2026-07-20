//! `aura fmt`: indentation canonicalization (SPEC §7.2).
//!
//! The formatter is line-oriented: line text (including comments and
//! intra-line alignment) is preserved, only indentation is normalized
//! (2 spaces per level), along with trailing whitespace and blank lines
//! (at most one in a row, none at the start/end of the file).
//!
//! Nesting depth is computed from tokens: `domain`/`component`/`def`/`type`/
//! `new`/`->`/`[`/`(` and a trailing `key:` open a level; `end`/`]`/`)`
//! close one. Safety invariant: the token stream never changes (see tests).

use crate::error::Diagnostic;
use crate::lexer::{Lexer, TokenKind};

const INDENT: &str = "  ";

#[derive(Default, Clone, Copy)]
struct LineInfo {
    /// Net depth change after this line.
    delta: i32,
    /// Minimum prefix sum within the line: a leading `end`/`]` dedents this very line.
    min_prefix: i32,
    /// The last significant token is `:` (opens an object block on the next line).
    ends_with_colon: bool,
    /// The last significant token is `->` — a trailing lambda arrow that opens a block.
    ends_with_arrow: bool,
    /// The line breaks off mid-expression (a trailing `,` `=` `.` `?` or binary
    /// operator) — the next line gets a continuation indent (+1).
    continues: bool,
}

pub fn format_source(src: &str) -> Result<String, Diagnostic> {
    let tokens = Lexer::new(src, 0).tokenize()?;

    // Line starts (byte offsets) for mapping a span to a line number
    let mut line_starts = vec![0usize];
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    let line_of = |off: u32| line_starts.partition_point(|s| *s <= off as usize) - 1;

    // Pre-scan: does a line contain an `end`? An arrow on such a line is an inline
    // lambda (`(x) -> y end`) and opens a block that closes on the same line; an arrow
    // on a line without `end` is either a trailing lambda opener (multi-line) or a cond
    // arm (`x -> y`), neither of which should raise the running depth mid-line.
    let mut line_has_end = vec![false; line_starts.len()];
    for t in &tokens {
        if matches!(t.kind, TokenKind::End) {
            line_has_end[line_of(t.span.start)] = true;
        }
    }

    let mut infos = vec![LineInfo::default(); line_starts.len()];
    for t in &tokens {
        let line = line_of(t.span.start);
        let d: i32 = match t.kind {
            TokenKind::Domain
            | TokenKind::Component
            | TokenKind::Def
            | TokenKind::Type
            | TokenKind::New
            | TokenKind::Cond
            | TokenKind::LBracket
            | TokenKind::LParen => 1,
            TokenKind::End | TokenKind::RBracket | TokenKind::RParen => -1,
            // Arrow raises depth in the running sum only for an inline lambda (line has `end`).
            TokenKind::Arrow if line_has_end[line] => 1,
            TokenKind::Newline | TokenKind::Eof => continue,
            _ => 0,
        };
        let info = &mut infos[line];
        info.delta += d;
        info.min_prefix = info.min_prefix.min(info.delta);
        info.ends_with_colon = matches!(t.kind, TokenKind::Colon);
        // A trailing `->` (last significant token, no `end` on the line) opens a block.
        info.ends_with_arrow = matches!(t.kind, TokenKind::Arrow) && !line_has_end[line];
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
            // at most one blank line, and never at the start of the file
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
        depth += info.delta + i32::from(info.ends_with_colon) + i32::from(info.ends_with_arrow);
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
        let src = "# heading\nbase_port        = 8000  # alignment is preserved\n";
        assert_eq!(format_source(src).unwrap(), src);
    }

    /// Safety invariant: formatting never changes the token stream.
    #[test]
    fn token_stream_is_preserved() {
        let formatted = format_source(MANIFEST).unwrap();
        assert_eq!(
            kinds(MANIFEST),
            kinds(&formatted),
            "fmt must not change semantics"
        );
    }

    /// Idempotence: running it again changes nothing.
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

    #[test]
    fn cond_block_indents_arms_flat() {
        // cond opens a block; arm arrows are inline and must NOT create indent creep.
        let messy = "t: cond\na == 1 -> \"x\"\n  b == 2 -> \"y\"\nelse -> \"z\"\nend\n";
        let want = "t: cond\n  a == 1 -> \"x\"\n  b == 2 -> \"y\"\n  else -> \"z\"\nend\n";
        assert_eq!(format_source(messy).unwrap(), want);
    }
}
