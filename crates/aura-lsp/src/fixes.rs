//! Quick fixes: the remedy a diagnostic's help names, applied in one keystroke.
//!
//! Only for mistakes whose repair is certain. A missing comma between two
//! fields on one line (D38) has exactly one reading - the parser already knows
//! a new field starts there - and so does a comma right before `end`. Where
//! the repair is a guess, the help line says what to do and the editor does
//! not presume to.

use aura_lang::lexer::{Lexer, TokenKind};
use aura_lang::parser::Parser;
use lsp_types::{Position, TextEdit};

use crate::diagnostics::LineIndex;

/// One fix: what the editor shows, the line of the diagnostic it repairs, and
/// the edit.
#[derive(Debug, PartialEq)]
pub struct Fix {
    pub title: &'static str,
    pub line: u32,
    pub edit: TextEdit,
}

/// The fix for the file's syntax error, if it is one of the certain kinds.
/// A file stops at its first syntax error, so there is at most one.
pub fn quick_fixes(text: &str) -> Vec<Fix> {
    let Ok(toks) = Lexer::new(text, 0).tokenize() else {
        return Vec::new();
    };
    let Err(diags) = Parser::new(toks.clone()).parse_module() else {
        return Vec::new();
    };
    let Some(d) = diags.first().filter(|d| d.code == "E0205") else {
        return Vec::new();
    };
    let at = d.primary.0.start;
    let Some(i) = toks.iter().position(|t| t.span.start == at) else {
        return Vec::new();
    };
    let index = LineIndex::new(text);
    let pos = |off: u32| index.position(text, off as usize);
    let line = pos(at).line;
    // The parser's own wording tells the two cases apart; a test runs the real
    // parser, so a reworded message fails there rather than going quiet here.
    match (d.message.as_str(), i.checked_sub(1).map(|p| &toks[p])) {
        ("expected a comma between fields", Some(prev)) => {
            // Right after the value, so `a: 1 b: 2` becomes `a: 1, b: 2`.
            let after = pos(prev.span.end);
            vec![Fix {
                title: "Insert the missing comma",
                line,
                edit: TextEdit {
                    range: lsp_types::Range {
                        start: after,
                        end: after,
                    },
                    new_text: ",".into(),
                },
            }]
        }
        ("a comma before `end`", Some(comma)) if matches!(comma.kind, TokenKind::Comma) => {
            // The comma and the space after it: `a: 1, end` becomes `a: 1 end`.
            vec![Fix {
                title: "Remove the comma before `end`",
                line,
                edit: TextEdit {
                    range: lsp_types::Range {
                        start: pos(comma.span.start),
                        end: Position {
                            line,
                            character: pos(at).character,
                        },
                    },
                    new_text: " ".into(),
                },
            }]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply a single-line edit and return the text.
    fn apply(text: &str, f: &Fix) -> String {
        let index = LineIndex::new(text);
        let (s, e) = (f.edit.range.start, f.edit.range.end);
        let (s, e) = (
            index.offset(text, s.line, s.character),
            index.offset(text, e.line, e.character),
        );
        format!("{}{}{}", &text[..s], f.edit.new_text, &text[e..])
    }

    fn parses(text: &str) -> bool {
        let toks = Lexer::new(text, 0).tokenize().expect("lexes");
        Parser::new(toks).parse_module().is_ok()
    }

    #[test]
    fn a_missing_comma_is_inserted_after_the_value() {
        let text = "x: 1\ne: new E path: \"/\" method: \"POST\" end\n";
        let fixes = quick_fixes(text);
        assert_eq!(fixes.len(), 1, "{fixes:?}");
        assert_eq!(fixes[0].line, 1);
        let fixed = apply(text, &fixes[0]);
        assert_eq!(fixed, "x: 1\ne: new E path: \"/\", method: \"POST\" end\n");
        assert!(parses(&fixed));
    }

    #[test]
    fn a_comma_before_end_is_removed() {
        let text = "l = cpu: 1, memory: 2, end\n";
        let fixes = quick_fixes(text);
        assert_eq!(fixes.len(), 1, "{fixes:?}");
        let fixed = apply(text, &fixes[0]);
        assert_eq!(fixed, "l = cpu: 1, memory: 2 end\n");
        assert!(parses(&fixed));
    }

    #[test]
    fn no_fix_for_an_error_whose_repair_is_a_guess() {
        // A statement left unseparated: the help explains, the editor does not
        // guess where the line should break.
        assert_eq!(quick_fixes("x: 1 y: 2\n"), Vec::new());
        assert_eq!(quick_fixes("x: 1\n"), Vec::new());
    }
}
