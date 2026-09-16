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

/// What an `end` (or a bracket) will close. The formatter never parses — that is
/// deliberate, so it can leave unparseable input alone rather than corrupt it —
/// but it does need to know which construct each opener belongs to, because `->`
/// means two different things and only its context tells them apart.
#[derive(PartialEq, Clone, Copy)]
enum Opener {
    /// `domain`, `def`, `type`, `enum`, `new`, and a trailing `key:`.
    Block,
    Cond,
    Lambda,
    Bracket,
    Paren,
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

    // The offset of the last non-trivia token on each line, so a token can ask
    // whether it ends its line. A trailing `key:` opens an object block, and the
    // opener stack has to record that or the `end` closing it pops the wrong
    // thing and every later question about what is open gets a stale answer.
    let mut last_tok_start = vec![u32::MAX; n];
    for t in &tokens {
        if !matches!(t.kind, TokenKind::Newline | TokenKind::Eof) {
            last_tok_start[line_of(t.span.start)] = t.span.start;
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
    // What is currently open, innermost last. A plain counter cannot say which
    // construct an `end` belongs to, and `->` needs exactly that: in a lambda it
    // opens a body that an `end` will close, in a `cond` arm it separates a
    // condition from a value and closes nothing. Guessing from nearby tokens got
    // both wrong — an arrow whose body opened a block (`-> new R`, closed by
    // `end end`) credited one level and closed two, and an arm holding an inline
    // lambda credited two and closed one.
    let mut open: Vec<Opener> = Vec::new();
    let mut arrow_seen_on_line: Vec<bool> = vec![false; n];
    for t in &tokens {
        if matches!(t.kind, TokenKind::Newline | TokenKind::Eof) {
            continue;
        }
        let line = line_of(t.span.start);
        toks_on[line].push(t);
        // An arrow separates a `cond` arm only when a `cond` is the innermost
        // thing open *and* it is the first arrow on the line: arms are
        // newline-separated, so any later arrow on the same line belongs to a
        // lambda inside the arm's value.
        let arrow_is_arm = matches!(t.kind, TokenKind::Arrow)
            && matches!(open.last(), Some(Opener::Cond))
            && !arrow_seen_on_line[line];
        if matches!(t.kind, TokenKind::Arrow) {
            arrow_seen_on_line[line] = true;
        }
        let d: i32 = match t.kind {
            TokenKind::Domain
            | TokenKind::Def
            | TokenKind::Type
            | TokenKind::Enum
            | TokenKind::New => {
                open.push(Opener::Block);
                1
            }
            TokenKind::Cond => {
                open.push(Opener::Cond);
                1
            }
            TokenKind::LBracket => {
                open.push(Opener::Bracket);
                1
            }
            TokenKind::LParen => {
                open.push(Opener::Paren);
                1
            }
            TokenKind::Arrow if !arrow_is_arm => {
                open.push(Opener::Lambda);
                1
            }
            TokenKind::End | TokenKind::RBracket | TokenKind::RParen => {
                open.pop();
                -1
            }
            // A `key:` at the end of its line opens an object block. It adds its
            // level below (via `ends_with_colon`), not here, but the stack must
            // still know, so the `end` that closes it pops this and not whatever
            // encloses it.
            TokenKind::Colon if t.span.start == last_tok_start[line] => {
                open.push(Opener::Block);
                0
            }
            _ => 0,
        };
        let info = &mut infos[line];
        info.delta += d;
        info.min_prefix = info.min_prefix.min(info.delta);
        info.ends_with_colon = matches!(t.kind, TokenKind::Colon);
        info.ends_with_arrow = matches!(t.kind, TokenKind::Arrow) && !arrow_is_arm;
        info.continues = matches!(
            t.kind,
            TokenKind::Comma
                | TokenKind::Assign
                | TokenKind::Dot
                // `?` is not a continuation: since D27 a line may end with it as
                // the nullable marker (`quota: Int?`), and treating that as an
                // unfinished expression indented everything after it. A ternary
                // broken across lines needs parentheses, and inside those the
                // continuation rule does not apply anyway.
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
        // The arrow's own level is already in `delta` — it is pushed like any
        // other opener now, so adding `ends_with_arrow` here would count it
        // twice. The flag survives only to tell `analyze_line` that a line
        // opening a block is not an alignment anchor.
        depth += info.delta + i32::from(info.ends_with_colon);
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
                    // Drop a trailing bare `\r`: `str::lines()` keeps it only at EOF,
                    // and re-emitting `\r\n` would then collapse it (non-idempotent).
                    Line::Verbatim(s) => out.push_str(s.strip_suffix('\r').unwrap_or(s)),
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
    fn a_lambda_whose_body_opens_a_block_keeps_the_rest_of_the_file_indented() {
        // `-> new R` opens two levels on one line and closes both on `end end`.
        // The flat depth counter credited only the `new`, so it went negative and
        // every line after the closing lost a level - including lines belonging
        // to entirely unrelated blocks further down.
        let src = concat!(
            "type R\n  v: Int\nend\n\n",
            "out:\n",
            "  items: [1, 2].map (x, i) -> new R\n",
            // Two openers on one line, so the body sits two levels in and the
            // pair of `end`s reads as the pair of closings it is. One level per
            // opener, with no special case for how many share a line.
            "      v: x\n",
            "  end end\n",
            "  tail: 2\n",
            "end\n",
            "after: 3\n",
        );
        assert_eq!(
            format_source(src).unwrap(),
            src,
            "already canonical input must come back unchanged"
        );
    }

    #[test]
    fn a_cond_arm_containing_a_lambda_does_not_shift_the_file() {
        // The other half of the same defect, and the one that was live before
        // anybody wrote `-> new`. Two arrows on one line: the arm separator and
        // the lambda. `line_has_end` credited a level to both, and only one
        // closed.
        let src = concat!(
            "f  = true\n",
            "xs = [1]\n",
            "a: cond\n",
            "  f -> xs.map (x, i) -> x end\n",
            "  else -> xs\n",
            "end\n",
            "z: 1\n",
        );
        assert_eq!(format_source(src).unwrap(), src);
    }

    #[test]
    fn formatting_is_idempotent_for_both_arrow_shapes() {
        // The property that matters more than either case: whatever the first
        // pass decides, a second pass must agree with it.
        for src in [
            "out:\n  xs: [1].map (x, i) -> x end\nend\n",
            "f = (a) ->\n  a + 1\nend\n",
            "t: cond\n  true -> 1\n  else -> 2\nend\n",
        ] {
            let once = format_source(src).unwrap();
            let twice = format_source(&once).unwrap();
            assert_eq!(once, twice, "not idempotent for: {src:?}");
        }
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
    fn verbatim_line_with_trailing_cr_is_idempotent() {
        // Fuzz regression: a block-string terminator line ending in a bare `\r`
        // (no `\n`) is emitted verbatim; without stripping it, `\r\n` on the first
        // pass collapses on the second -> non-idempotent.
        let once = format_source("k: text\n  body\nend\r").unwrap();
        assert!(!once.contains('\r'));
        assert_eq!(format_source(&once).unwrap(), once);
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
