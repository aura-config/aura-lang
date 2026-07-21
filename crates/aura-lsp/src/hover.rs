//! Hover: the identifier under the cursor, looked up in the stdlib surface.

use crate::stdlib::Stdlib;

/// Markdown for the identifier at `(line, character)` (both LSP/UTF-16), or
/// `None` if there is no word there or it is not a known method/builtin.
pub fn hover_markdown(text: &str, line: u32, character: u32, stdlib: &Stdlib) -> Option<String> {
    let word = word_at(text, line, character)?;
    let matches: Vec<_> = stdlib.entries.iter().filter(|e| e.name == word).collect();
    if matches.is_empty() {
        return None;
    }
    let mut md = String::new();
    for e in matches {
        let sig = if e.receiver == "builtins" {
            e.signature()
        } else {
            format!("{}.{}", e.receiver, e.signature())
        };
        md.push_str(&format!("```aura\n{sig}\n```\n{}\n\n", e.doc));
    }
    Some(md.trim_end().to_string())
}

/// The identifier (`[A-Za-z0-9_]+`) covering a UTF-16 position, if any.
fn word_at(text: &str, line: u32, character: u32) -> Option<String> {
    let line_str = text.lines().nth(line as usize)?;
    // UTF-16 character offset -> byte index within the line.
    let mut byte = line_str.len();
    let mut u16 = 0u32;
    for (i, ch) in line_str.char_indices() {
        if u16 >= character {
            byte = i;
            break;
        }
        u16 += ch.len_utf16() as u32;
    }
    let bytes = line_str.as_bytes();
    let is_id = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut start = byte;
    while start > 0 && is_id(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = byte;
    while end < bytes.len() && is_id(bytes[end]) {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some(line_str[start..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sl() -> Stdlib {
        Stdlib::load()
    }

    #[test]
    fn hover_on_a_method_shows_signature_and_doc() {
        // cursor on `map` in `xs.map`
        let md = hover_markdown("out: xs.map\n", 0, 9, &sl()).expect("hover");
        assert!(md.contains("List.map(fn) -> List"), "{md}");
        assert!(md.to_lowercase().contains("each element"));
    }

    #[test]
    fn hover_on_a_builtin_has_no_receiver_prefix() {
        let md = hover_markdown("n: range\n", 0, 4, &sl()).expect("hover");
        assert!(md.contains("range(n) -> List"), "{md}");
        assert!(!md.contains("builtins."), "{md}");
    }

    #[test]
    fn hover_on_unknown_word_is_none() {
        assert!(hover_markdown("foo: bar\n", 0, 5, &sl()).is_none());
    }

    #[test]
    fn word_boundaries_with_cursor_after_word() {
        // caret just past the last char of `trim`
        assert_eq!(word_at("s.trim\n", 0, 6).as_deref(), Some("trim"));
    }
}
