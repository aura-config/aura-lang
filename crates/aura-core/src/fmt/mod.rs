//! `aura fmt`: canonical formatter (SPEC §7.2).
//!
//! Three things happen, all whitespace-only (the token stream never changes):
//! 1. **Indentation** is normalized to 2 spaces per level.
//! 2. **Intra-line spacing** is canonicalized: runs of spaces/tabs between tokens
//!    collapse to a single space (strings and trailing comments are untouched).
//! 3. **Column alignment**: consecutive `name = value`, `key: value` and `cond`
//!    arms (`cond -> value`) are aligned in columns, together with their trailing
//!    `# comments`. A blank line, a comment line, a different construct or an
//!    indent change ends a run. The `else` arm is left unaligned.
//!
//! Block-string interiors (D16) are emitted verbatim.

use crate::error::Diagnostic;
use crate::lexer::{Lexer, Token, TokenKind};

const INDENT: &str = "  ";

#[derive(Default, Clone, Copy)]
struct LineInfo {
    delta: i32,
    min_prefix: i32,
    ends_with_colon: bool,
    ends_with_arrow: bool,
    continues: bool,
}

/// One anchor kind for column alignment.
#[derive(PartialEq, Clone, Copy)]
enum Kind {
    Assign, // name = value
    Colon,  // key: value
    Arrow,  // cond -> value
}

enum Body {
    Plain(String),
    Anchored {
        kind: Kind,
        field: String,
        joiner: &'static str,
        rest: String,
    },
}

struct CodeLine {
    level: usize,
    /// Filled by the anchor pass; holds the aligned code without the comment.
    code: String,
    body: Body,
    comment: Option<String>,
}

enum Line {
    Blank,
    Verbatim(String),
    Comment { level: usize, text: String },
    Code(CodeLine),
}

pub fn format_source(src: &str) -> Result<String, Diagnostic> {
    let tokens = Lexer::new(src, 0).tokenize()?;

    let mut line_starts = vec![0usize];
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    let n = line_starts.len();
    let line_of = |off: u32| line_starts.partition_point(|s| *s <= off as usize) - 1;
    let line_end = |idx: usize| -> usize {
        if idx + 1 < line_starts.len() {
            line_starts[idx + 1] - 1
        } else {
            src.len()
        }
    };

    // A line with an `end`: an arrow on it is an inline lambda (opens+closes there).
    let mut line_has_end = vec![false; n];
    for t in &tokens {
        if matches!(t.kind, TokenKind::End) {
            line_has_end[line_of(t.span.start)] = true;
        }
    }

    // D16 block strings are single multi-line string tokens; their interior lines
    // are emitted verbatim (only the opener line is formatted).
    let mut verbatim_line = vec![false; n];
    for t in &tokens {
        if matches!(t.kind, TokenKind::Str(_) | TokenKind::InterpStr(_)) {
            let start = line_of(t.span.start);
            let end = line_of(t.span.end.saturating_sub(1));
            for ln in verbatim_line.iter_mut().take(end + 1).skip(start + 1) {
                *ln = true;
            }
        }
    }

    // Depth deltas per line + the "opens/continues a block" flags.
    let mut infos = vec![LineInfo::default(); n];
    // Tokens on each physical line (excluding Newline/Eof), in order.
    let mut toks_on: Vec<Vec<&Token>> = vec![Vec::new(); n];
    for t in &tokens {
        if matches!(t.kind, TokenKind::Newline | TokenKind::Eof) {
            continue;
        }
        let line = line_of(t.span.start);
        toks_on[line].push(t);
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
            TokenKind::Arrow if line_has_end[line] => 1,
            _ => 0,
        };
        let info = &mut infos[line];
        info.delta += d;
        info.min_prefix = info.min_prefix.min(info.delta);
        info.ends_with_colon = matches!(t.kind, TokenKind::Colon);
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

    // Pass 0: classify and canonically render each physical line.
    let mut lines: Vec<Line> = Vec::with_capacity(n);
    let mut depth: i32 = 0;
    let mut prev_continues = false;
    for (idx, raw) in src.lines().enumerate() {
        if verbatim_line[idx] {
            lines.push(Line::Verbatim(raw.to_string()));
            continue;
        }
        if raw.trim().is_empty() {
            lines.push(Line::Blank);
            continue;
        }
        let info = infos[idx];
        let level = (depth + info.min_prefix.min(0) + i32::from(prev_continues)).max(0) as usize;
        depth += info.delta + i32::from(info.ends_with_colon) + i32::from(info.ends_with_arrow);
        prev_continues = info.continues;

        let toks = &toks_on[idx];
        if toks.is_empty() {
            lines.push(Line::Comment {
                level,
                text: raw.trim().to_string(),
            });
            continue;
        }
        let le = line_end(idx);
        let comment = trailing_comment(src, toks, le);
        let body = analyze_line(src, toks, le, &info);
        let code = match &body {
            Body::Plain(s) => s.clone(),
            Body::Anchored { .. } => String::new(), // filled by the anchor pass
        };
        lines.push(Line::Code(CodeLine {
            level,
            code,
            body,
            comment,
        }));
    }

    align_anchors(&mut lines);
    align_comments(&mut lines);

    // Emit with indentation and blank-line normalization.
    let mut out = String::with_capacity(src.len());
    let mut pending_blank = false;
    let mut wrote_any = false;
    for line in &lines {
        match line {
            Line::Blank => {
                pending_blank = wrote_any;
            }
            other => {
                if pending_blank {
                    out.push('\n');
                    pending_blank = false;
                }
                match other {
                    Line::Verbatim(s) => out.push_str(s),
                    Line::Comment { level, text } => {
                        push_indent(&mut out, *level);
                        out.push_str(text);
                    }
                    Line::Code(c) => {
                        push_indent(&mut out, c.level);
                        out.push_str(&c.code);
                    }
                    Line::Blank => unreachable!(),
                }
                out.push('\n');
                wrote_any = true;
            }
        }
    }

    // Backstop: formatting is whitespace-only and must never change the token
    // stream. On a pathological input where the added trailing newline or a
    // collapsed gap would (e.g. a bare `text` RHS becoming a D16 block opener),
    // leave the file untouched rather than corrupt it.
    if token_shape(&out) != Some(token_shape_of(&tokens)) {
        return Ok(src.to_string());
    }
    Ok(out)
}

/// Non-trivia token kinds as debug strings (lifetime-free, comparable across inputs).
fn token_shape_of(tokens: &[Token]) -> Vec<String> {
    tokens
        .iter()
        .filter(|t| !matches!(t.kind, TokenKind::Newline | TokenKind::Eof))
        .map(|t| format!("{:?}", t.kind))
        .collect()
}

fn token_shape(src: &str) -> Option<Vec<String>> {
    Lexer::new(src, 0)
        .tokenize()
        .ok()
        .map(|ts| token_shape_of(&ts))
}

fn push_indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str(INDENT);
    }
}

/// The `#…` trailing comment on a line, if any (never inside a multi-line token).
fn trailing_comment(src: &str, toks: &[&Token], line_end: usize) -> Option<String> {
    let last = toks.last()?;
    if (last.span.end as usize) > line_end {
        return None; // a block-string opener: no room for a comment
    }
    let tail = src[last.span.end as usize..line_end].trim();
    tail.starts_with('#').then(|| tail.to_string())
}

/// A token's text, truncated to its own line (a multi-line block-string token
/// contributes only its opener word, e.g. `text`).
fn tok_text<'s>(src: &'s str, t: &Token, line_end: usize) -> &'s str {
    let end = (t.span.end as usize).min(line_end);
    &src[t.span.start as usize..end]
}

/// Join tokens with canonical spacing: one space where the source had any
/// whitespace between two tokens, none otherwise. Collapses runs and tabs.
fn render_core(src: &str, toks: &[&Token], line_end: usize) -> String {
    let mut s = String::new();
    for (i, t) in toks.iter().enumerate() {
        if i > 0 {
            // Any trivia between two tokens (space, tab, or a bare `\r` that
            // `src.lines()` did not treat as a break) means they were separated,
            // so emit exactly one space. Never zero — that could fuse two tokens.
            let gap = &src[toks[i - 1].span.end as usize..t.span.start as usize];
            if !gap.is_empty() {
                s.push(' ');
            }
        }
        s.push_str(tok_text(src, t, line_end));
    }
    s
}

/// Decide a line's alignment anchor (or none). Block-opener lines never anchor.
fn analyze_line(src: &str, toks: &[&Token], line_end: usize, info: &LineInfo) -> Body {
    let plain = || Body::Plain(render_core(src, toks, line_end));
    if info.ends_with_colon || info.ends_with_arrow {
        return plain();
    }
    let pos = |k: fn(&TokenKind) -> bool| toks.iter().position(|t| k(&t.kind));

    if let Some(i) = pos(|k| matches!(k, TokenKind::Assign)) {
        if i + 1 < toks.len() {
            return Body::Anchored {
                kind: Kind::Assign,
                field: render_core(src, &toks[..i], line_end),
                joiner: " = ",
                rest: render_core(src, &toks[i + 1..], line_end),
            };
        }
    }
    if let Some(i) = pos(|k| matches!(k, TokenKind::Colon)) {
        if i > 0 && i + 1 < toks.len() {
            return Body::Anchored {
                kind: Kind::Colon,
                field: render_core(src, &toks[..i], line_end) + ":",
                joiner: " ",
                rest: render_core(src, &toks[i + 1..], line_end),
            };
        }
    }
    if let Some(i) = pos(|k| matches!(k, TokenKind::Arrow)) {
        let field = render_core(src, &toks[..i], line_end);
        if i + 1 < toks.len() && field != "else" && !matches!(toks[0].kind, TokenKind::Def) {
            return Body::Anchored {
                kind: Kind::Arrow,
                field,
                joiner: " -> ",
                rest: render_core(src, &toks[i + 1..], line_end),
            };
        }
    }
    plain()
}

/// Column-align consecutive anchored lines of the same kind and indent level.
fn align_anchors(lines: &mut [Line]) {
    let mut i = 0;
    while i < lines.len() {
        let (kind, level) = match &lines[i] {
            Line::Code(CodeLine {
                body: Body::Anchored { kind, .. },
                level,
                ..
            }) => (*kind, *level),
            _ => {
                i += 1;
                continue;
            }
        };
        // Extend the run over same-kind, same-level anchored lines.
        let mut j = i;
        let mut width = 0usize;
        while j < lines.len() {
            match &lines[j] {
                Line::Code(CodeLine {
                    body: Body::Anchored { kind: k, field, .. },
                    level: l,
                    ..
                }) if *k == kind && *l == level => {
                    width = width.max(field.chars().count());
                    j += 1;
                }
                _ => break,
            }
        }
        for line in &mut lines[i..j] {
            if let Line::Code(c) = line {
                if let Body::Anchored {
                    field,
                    joiner,
                    rest,
                    ..
                } = &c.body
                {
                    let pad = width - field.chars().count();
                    c.code = format!("{field}{}{joiner}{rest}", " ".repeat(pad));
                }
            }
        }
        i = j;
    }
}

/// Align trailing comments within runs of consecutive code lines at one level.
fn align_comments(lines: &mut [Line]) {
    let mut i = 0;
    while i < lines.len() {
        let level = match &lines[i] {
            Line::Code(CodeLine { level, .. }) => *level,
            _ => {
                i += 1;
                continue;
            }
        };
        let mut j = i;
        let mut col = 0usize;
        while j < lines.len() {
            match &lines[j] {
                Line::Code(c) if c.level == level => {
                    if c.comment.is_some() {
                        col = col.max(c.code.chars().count() + 1);
                    }
                    j += 1;
                }
                _ => break,
            }
        }
        for line in &mut lines[i..j] {
            if let Line::Code(c) = line {
                if let Some(comment) = &c.comment {
                    let pad = col.saturating_sub(c.code.chars().count());
                    c.code = format!("{}{}{comment}", c.code, " ".repeat(pad));
                }
            }
        }
        i = j.max(i + 1);
    }
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
    fn aligns_assignments_and_comments() {
        let messy = "a = 1  # one\nlonger_name = 2\nc = 3 # three\n";
        let want = "a           = 1 # one\nlonger_name = 2\nc           = 3 # three\n";
        assert_eq!(format_source(messy).unwrap(), want);
    }

    #[test]
    fn aligns_properties_by_colon() {
        let messy = "sum:   1\nfloat_div:  2\n";
        let want = "sum:       1\nfloat_div: 2\n";
        assert_eq!(format_source(messy).unwrap(), want);
    }

    #[test]
    fn aligns_cond_arrows_but_not_else() {
        let messy =
            "t: cond\n  region == \"eu-central\" -> \"a\"\n  region == \"us\" -> \"b\"\n  else -> \"c\"\nend\n";
        let want = "t: cond\n  region == \"eu-central\" -> \"a\"\n  region == \"us\"         -> \"b\"\n  else -> \"c\"\nend\n";
        assert_eq!(format_source(messy).unwrap(), want);
    }

    #[test]
    fn backstop_leaves_token_changing_input_untouched() {
        // Fuzz regression: `x =text` (no newline) has `text` as a plain ident;
        // adding a trailing newline would make it a D16 block opener. The backstop
        // returns the input unchanged rather than corrupt the token stream.
        let src = "x =text";
        assert_eq!(format_source(src).unwrap(), src);
    }

    #[test]
    fn bare_cr_between_tokens_does_not_fuse_them() {
        // Fuzz regression: a bare `\r` is trivia to the lexer but not a line break
        // to src.lines(); the two idents must stay separated, and the output must
        // re-lex identically (idempotence).
        let once = format_source("a\rb\n").unwrap();
        assert_eq!(once, "a b\n");
        assert_eq!(format_source(&once).unwrap(), once);
    }

    #[test]
    fn collapses_extra_spaces_outside_strings() {
        assert_eq!(
            format_source("x = \"a    b\"  +  1\n").unwrap(),
            "x = \"a    b\" + 1\n"
        );
    }

    #[test]
    fn blank_line_breaks_an_alignment_run() {
        let src = "a = 1\nbb = 2\n\nc = 3\n";
        assert_eq!(format_source(src).unwrap(), "a  = 1\nbb = 2\n\nc = 3\n");
    }

    #[test]
    fn normalizes_indentation() {
        let messy = "domain \"d\"\n      x: 1\n  security:\n        tls: true\n  end\nend\n";
        let want = "domain \"d\"\n  x: 1\n  security:\n    tls: true\n  end\nend\n";
        assert_eq!(format_source(messy).unwrap(), want);
    }

    #[test]
    fn block_string_interior_is_preserved() {
        let messy =
            "domain \"d\"\n      script: text\n    #!/bin/sh\n    echo hi\n  end\nx: 1\nend\n";
        let want = "domain \"d\"\n  script: text\n    #!/bin/sh\n    echo hi\n  end\n  x: 1\nend\n";
        assert_eq!(format_source(messy).unwrap(), want);
        assert_eq!(kinds(messy), kinds(&format_source(messy).unwrap()));
    }

    /// Safety invariant: formatting never changes the token stream.
    #[test]
    fn token_stream_is_preserved() {
        let formatted = format_source(MANIFEST).unwrap();
        assert_eq!(kinds(MANIFEST), kinds(&formatted), "fmt changed semantics");
    }

    /// Idempotence: running it again changes nothing.
    #[test]
    fn is_idempotent() {
        let once = format_source(MANIFEST).unwrap();
        let twice = format_source(&once).unwrap();
        assert_eq!(once, twice);
    }
}
