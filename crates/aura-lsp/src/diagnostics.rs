//! Turns Aura source text into LSP diagnostics via lex → parse → analysis.
//!
//! This is the fast, I/O-free layer (the `aura check` pipeline): it never
//! evaluates, so it is safe to run on every keystroke. Byte offsets from the
//! core's spans are mapped to LSP positions in UTF-16 code units (the default
//! LSP position encoding).

use aura_core::analysis::analyze;
use aura_core::error::{Diagnostic as CoreDiagnostic, Severity};
use aura_core::lexer::Lexer;
use aura_core::parser::Parser;
use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range};

/// Maps byte offsets to `(line, utf16-character)` positions for one document.
pub struct LineIndex {
    /// Byte offset at the start of each line.
    line_starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        LineIndex { line_starts }
    }

    /// LSP `Position` for a byte offset. Offsets are clamped to the text and to
    /// a UTF-8 char boundary so a mid-character span can never panic.
    pub fn position(&self, text: &str, offset: usize) -> Position {
        let mut offset = offset.min(text.len());
        while offset > 0 && !text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self.line_starts.partition_point(|&s| s <= offset) - 1;
        let line_start = self.line_starts[line];
        let character = text[line_start..offset].encode_utf16().count() as u32;
        Position {
            line: line as u32,
            character,
        }
    }

    /// Byte offset for an LSP `(line, character)` position (UTF-16 character).
    /// Clamps out-of-range lines/characters to the document.
    pub fn offset(&self, text: &str, line: u32, character: u32) -> usize {
        let Some(&line_start) = self.line_starts.get(line as usize) else {
            return text.len();
        };
        let mut u16 = 0u32;
        for (i, ch) in text[line_start..].char_indices() {
            if u16 >= character || ch == '\n' {
                return line_start + i;
            }
            u16 += ch.len_utf16() as u32;
        }
        text.len()
    }
}

/// Runs the static pipeline and returns LSP diagnostics (empty if the source is clean).
pub fn analyze_source(text: &str) -> Vec<Diagnostic> {
    let index = LineIndex::new(text);
    let tokens = match Lexer::new(text, 0).tokenize() {
        Ok(t) => t,
        Err(d) => return vec![to_lsp(&d, text, &index)],
    };
    let module = match Parser::new(tokens).parse_module() {
        Ok(m) => m,
        // The parser recovers and can report several errors at once.
        Err(ds) => return ds.iter().map(|d| to_lsp(d, text, &index)).collect(),
    };
    // A buffer is analyzed as a root module (full checks, like `aura check`).
    analyze(&module, true)
        .iter()
        .map(|d| to_lsp(d, text, &index))
        .collect()
}

fn to_lsp(d: &CoreDiagnostic, text: &str, index: &LineIndex) -> Diagnostic {
    let (span, _label) = &d.primary;
    let range = Range {
        start: index.position(text, span.start as usize),
        end: index.position(text, span.end as usize),
    };
    let severity = Some(match d.severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
    });
    let message = match &d.help {
        Some(help) => format!("{}\n\nhelp: {help}", d.message),
        None => d.message.clone(),
    };
    Diagnostic {
        range,
        severity,
        code: Some(NumberOrString::String(d.code.to_string())),
        source: Some("aura".to_string()),
        message,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_source_has_no_diagnostics() {
        assert!(analyze_source("port: 8080\nname: \"api\"\n").is_empty());
    }

    fn find<'a>(ds: &'a [Diagnostic], code: &str) -> &'a Diagnostic {
        ds.iter()
            .find(|d| d.code == Some(NumberOrString::String(code.into())))
            .unwrap_or_else(|| panic!("no {code} in {ds:?}"))
    }

    #[test]
    fn redefinition_is_reported_with_code_and_range() {
        // Reassigning `x` in the same scope yields E0301 (alongside an unused warning).
        let ds = analyze_source("x = 1\nx = 2\n");
        let e = find(&ds, "E0301");
        // The error is on the second line (0-based line 1).
        assert_eq!(e.range.start.line, 1);
    }

    #[test]
    fn parse_error_produces_one_diagnostic() {
        let ds = analyze_source("x = \n");
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn utf16_columns_for_astral_chars() {
        // A 4-byte emoji is 2 UTF-16 units; a redefinition on the next line must
        // still map to line 1 regardless of the astral char above it.
        let ds = analyze_source("s = \"😀\"\ns = 2\n");
        assert_eq!(find(&ds, "E0301").range.start.line, 1);
    }
}
