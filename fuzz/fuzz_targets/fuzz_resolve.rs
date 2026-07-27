#![no_main]
//! Fuzz name resolution. It does span arithmetic (interpolation offsets are
//! recovered from subslices and biased into absolute positions), so a malformed
//! manifest must not produce an out-of-bounds span or a panic. Every span the
//! resolver reports is asserted to be a real char boundary in the source —
//! otherwise a consumer slicing by it would panic instead.

use libfuzzer_sys::fuzz_target;

use aura_lang::lexer::Lexer;
use aura_lang::parser::Parser;
use aura_lang::resolve::resolve;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(toks) = Lexer::new(src, 0).tokenize() else {
        return;
    };
    let Ok(module) = Parser::new(toks).parse_module() else {
        return;
    };
    let r = resolve(src, &module);

    let check = |what: &str, s: aura_lang::span::Span| {
        let (start, end) = (s.start as usize, s.end as usize);
        assert!(start <= end, "{what}: inverted span {start}..{end}");
        assert!(end <= src.len(), "{what}: span {start}..{end} past the source");
        assert!(
            src.is_char_boundary(start) && src.is_char_boundary(end),
            "{what}: span {start}..{end} is not on char boundaries"
        );
    };
    for b in &r.bindings {
        check("binding decl", b.decl);
        check("binding scope", b.scope);
    }
    for u in &r.uses {
        check("use", u.span);
        // A resolved use must point at a real binding.
        if let Some(i) = u.binding {
            assert!(i < r.bindings.len(), "use resolved to a missing binding");
        }
    }
    // Occurrences of every binding must be slicable and self-consistent.
    for i in 0..r.bindings.len() {
        for s in r.occurrences(i) {
            check("occurrence", s);
        }
    }
});
