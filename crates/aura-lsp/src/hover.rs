//! Hover: the identifier under the cursor, looked up in the stdlib surface.

use crate::stdlib::Stdlib;

/// Markdown for the identifier at `(line, character)` (both LSP/UTF-16), or
/// `None` if there is no word there or it is not a known method/builtin.
pub fn hover_markdown(text: &str, line: u32, character: u32, stdlib: &Stdlib) -> Option<String> {
    if let Some(md) = host_import_markdown(text, line) {
        return Some(md);
    }
    if let Some(md) = binding_markdown(text, line, character) {
        return Some(md);
    }
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

/// A name bound with `=`: say that it is not in the output (D10). `=` and
/// `:` read alike, and since D38 they take the same blocks, so the one thing
/// that tells them apart is worth stating where the name is used.
fn binding_markdown(text: &str, line: u32, character: u32) -> Option<String> {
    let def = crate::goto::definition_range(text, line, character)?;
    let index = crate::diagnostics::LineIndex::new(text);
    let start = index.offset(text, def.start.line, def.start.character);
    let end = index.offset(text, def.end.line, def.end.character);
    let rest = text[end..].trim_start_matches([' ', '\t']);
    if !rest.starts_with('=') || rest.starts_with("==") {
        return None;
    }
    let name = &text[start..end];
    Some(format!(
        "`{name}` is bound with `=`: usable in this file, not part of the output. \
         Write `{name}:` to output it (D10)."
    ))
}

/// D35 §7: on an `import "host:..."` line, say where the module comes from -
/// not from a file, so there is nothing to find - and which of its functions
/// this file calls, the same list `aura check` prints.
fn host_import_markdown(text: &str, line: u32) -> Option<String> {
    let toks = aura_lang::lexer::Lexer::new(text, 0).tokenize().ok()?;
    let module = aura_lang::parser::Parser::new(toks).parse_module().ok()?;
    let index = crate::diagnostics::LineIndex::new(text);
    let (alias, name, _) = aura_lang::host::host_imports(&module)
        .into_iter()
        .find(|(_, _, span)| index.position(text, span.start as usize).line == line)?;
    let mut md = format!(
        "```aura
import \"host:{name}\" as {alias}
```
         A host module: provided by the application that embeds this script, at          run time - not by a file. `aura eval` has no host, so it stops with          E0007; `aura check` lists what the script needs."
    );
    let mut calls: Vec<String> = Vec::new();
    for r in aura_lang::host::requirements(&module) {
        if r.module != name {
            continue;
        }
        let c = match r.func {
            Some(f) => format!("`{alias}.{f}`"),
            None => format!("`{alias}` (used as a value)"),
        };
        if !calls.contains(&c) {
            calls.push(c);
        }
    }
    if !calls.is_empty() {
        md.push_str(&format!(
            "

This file calls: {}.",
            calls.join(", ")
        ));
    }
    Some(md)
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
    fn hover_on_a_host_import_explains_where_it_comes_from() {
        let src = "import \"host:geo\" as geo
r: geo.region(\"x\")
";
        let md = hover_markdown(src, 0, 3, &sl()).expect("hover");
        assert!(md.contains("provided by the application"), "{md}");
        assert!(md.contains("`geo.region`"), "{md}");
        // Only the import line explains the module.
        assert!(hover_markdown(src, 1, 0, &sl()).is_none());
    }

    #[test]
    fn hover_on_a_name_bound_with_equals_says_it_is_not_output() {
        let src = "defaults =\n  port: 1\nend\nlimits = cpu: 1 end\nout: defaults.port\n";
        // At a use, and at the binding itself.
        for (line, ch, name) in [(4, 6, "defaults"), (0, 2, "defaults"), (3, 1, "limits")] {
            let md = hover_markdown(src, line, ch, &sl()).expect("hover");
            assert!(md.contains(&format!("`{name}` is bound with `=`")), "{md}");
        }
        // A property is output, so it gets no such note.
        assert!(hover_markdown("out: 1\ny: out\n", 1, 4, &sl()).is_none());
    }

    #[test]
    fn word_boundaries_with_cursor_after_word() {
        // caret just past the last char of `trim`
        assert_eq!(word_at("s.trim\n", 0, 6).as_deref(), Some("trim"));
    }
}
