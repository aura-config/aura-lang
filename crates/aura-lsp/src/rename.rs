//! Rename.
//!
//! Renaming a config is not a cosmetic refactor: a rename that silently rewrites
//! the wrong occurrence changes what gets deployed. So this is the one feature
//! that never guesses — it works only off `aura_lang::resolve` (scope precise) and
//! **refuses** whenever it cannot prove the edit is safe. A refusal costs the user
//! one manual edit; a wrong rename costs them an incident.

use aura_lang::lexer::token::KEYWORDS;
use lsp_types::Range;

use crate::diagnostics::LineIndex;
use crate::goto::resolution;

/// Why a rename was refused. The message is shown to the user verbatim.
#[derive(Debug, PartialEq)]
pub enum Refusal {
    /// The file does not parse, so scopes are unknown.
    Unparseable,
    /// Not a local binding: a method name, a schema field, a keyword, a literal.
    NotABinding,
    /// The new name is not a usable identifier.
    BadName(String),
    /// The new name is already bound in an overlapping scope, so the rename would
    /// change which binding some occurrence refers to.
    WouldCapture(String),
    /// `pub` items are the module's API; importers are not visible from here.
    IsPublic(String),
}

impl Refusal {
    pub fn message(&self) -> String {
        match self {
            Refusal::Unparseable => {
                "cannot rename while the file has syntax errors: scopes are unknown".into()
            }
            Refusal::NotABinding => {
                "not a renameable symbol (only variables, parameters, `def`, `type`, \
                 `enum` and import aliases can be renamed)"
                    .into()
            }
            Refusal::BadName(why) => format!("invalid new name: {why}"),
            Refusal::WouldCapture(name) => format!(
                "'{name}' is already bound in an overlapping scope — renaming would \
                 change which binding some occurrence refers to; pick another name"
            ),
            Refusal::IsPublic(name) => format!(
                "'{name}' is `pub`, so other modules may import it by name; rename it \
                 there first, or make it private"
            ),
        }
    }
}

/// Whether `name` can be used as a new identifier.
fn check_name(name: &str) -> Result<(), Refusal> {
    if name.is_empty() {
        return Err(Refusal::BadName("empty".into()));
    }
    if KEYWORDS.contains(&name) {
        return Err(Refusal::BadName(format!("'{name}' is a keyword")));
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(Refusal::BadName(
            "must start with a letter or underscore".into(),
        ));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(Refusal::BadName(
            "only letters, digits and underscores are allowed".into(),
        ));
    }
    Ok(())
}

/// The range the client should pre-fill in its rename prompt, or a refusal — this
/// is `textDocument/prepareRename`, so the user learns it is impossible *before*
/// typing a new name.
pub fn prepare(text: &str, line: u32, character: u32) -> Result<Range, Refusal> {
    let r = resolution(text).ok_or(Refusal::Unparseable)?;
    let index = LineIndex::new(text);
    let offset = index.offset(text, line, character) as u32;
    let b = r.binding_at(offset).ok_or(Refusal::NotABinding)?;
    let binding = &r.bindings[b];
    if binding.public {
        return Err(Refusal::IsPublic(binding.name.clone()));
    }
    let d = binding.decl;
    Ok(Range {
        start: index.position(text, d.start as usize),
        end: index.position(text, d.end as usize),
    })
}

/// Every range to replace with `new_name`, or a refusal. Ranges are disjoint and
/// in source order, so a client may apply them in any order.
pub fn edits(text: &str, line: u32, character: u32, new_name: &str) -> Result<Vec<Range>, Refusal> {
    check_name(new_name)?;
    let r = resolution(text).ok_or(Refusal::Unparseable)?;
    let index = LineIndex::new(text);
    let offset = index.offset(text, line, character) as u32;
    let b = r.binding_at(offset).ok_or(Refusal::NotABinding)?;
    let binding = &r.bindings[b];

    if binding.public {
        return Err(Refusal::IsPublic(binding.name.clone()));
    }
    if new_name == binding.name {
        return Ok(Vec::new()); // a no-op rename is not an error
    }
    // Capture check. Conservative on purpose: any binding of the new name in a
    // scope that overlaps this one is a refusal, even when no occurrence would
    // actually move. An import alias is file-local, so the same rule covers it.
    if !r.conflicting(new_name, binding.scope).is_empty() {
        return Err(Refusal::WouldCapture(new_name.to_string()));
    }

    Ok(r.occurrences(b)
        .into_iter()
        .map(|s| Range {
            start: index.position(text, s.start as usize),
            end: index.position(text, s.end as usize),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply the edits to `text` — the only honest way to test a rename.
    fn apply(text: &str, line: u32, ch: u32, new_name: &str) -> Result<String, Refusal> {
        let ranges = edits(text, line, ch, new_name)?;
        let index = LineIndex::new(text);
        // Back to front, so earlier offsets stay valid.
        let mut offsets: Vec<(usize, usize)> = ranges
            .iter()
            .map(|r| {
                (
                    index.offset(text, r.start.line, r.start.character),
                    index.offset(text, r.end.line, r.end.character),
                )
            })
            .collect();
        offsets.sort_unstable();
        let mut out = text.to_string();
        for (s, e) in offsets.into_iter().rev() {
            out.replace_range(s..e, new_name);
        }
        Ok(out)
    }

    #[test]
    fn renames_only_the_binding_under_the_cursor() {
        // The `x` parameter must not be touched when the top-level `x` is renamed.
        let src = "x = 1\ndef f(x)\n  y: x\nend\nz: x\n";
        assert_eq!(
            apply(src, 0, 0, "count").unwrap(),
            "count = 1\ndef f(x)\n  y: x\nend\nz: count\n"
        );
        // …and the reverse: renaming the parameter leaves the outer binding alone.
        assert_eq!(
            apply(src, 1, 6, "p").unwrap(),
            "x = 1\ndef f(p)\n  y: p\nend\nz: x\n"
        );
    }

    #[test]
    fn renames_inside_interpolation() {
        let src = "name = \"api\"\nid: \"#{name}-1\"\n";
        assert_eq!(
            apply(src, 0, 0, "svc").unwrap(),
            "svc = \"api\"\nid: \"#{svc}-1\"\n"
        );
    }

    #[test]
    fn renames_a_shadowing_binding_independently() {
        let src = "p = 1\ndomain \"d\"\n  shadow p = 2\n  a: p\nend\nb: p\n";
        // Rename the inner (shadowing) `p`; the outer one and `b: p` stay.
        assert_eq!(
            apply(src, 3, 5, "inner").unwrap(),
            "p = 1\ndomain \"d\"\n  shadow inner = 2\n  a: inner\nend\nb: p\n"
        );
    }

    #[test]
    fn renames_a_type_and_its_uses() {
        let src = "type Quota\n  cpu: Int\nend\nq: new Quota\n  cpu: 2\nend\n";
        assert_eq!(
            apply(src, 0, 5, "Limits").unwrap(),
            "type Limits\n  cpu: Int\nend\nq: new Limits\n  cpu: 2\nend\n"
        );
    }

    #[test]
    fn refuses_when_the_file_does_not_parse() {
        // Scopes are unknown mid-edit, so there is no safe answer.
        assert_eq!(
            edits("x = 1\ndef f(\ny: x\n", 0, 0, "y"),
            Err(Refusal::Unparseable)
        );
    }

    #[test]
    fn refuses_a_capture() {
        // `y` already exists in the def's scope, which is nested in `x`'s scope:
        // renaming `x` to `y` would make `z: x` resolve differently.
        let src = "x = 1\ndef f()\n  y = 2\n  w: y\nend\nz: x\n";
        assert_eq!(
            edits(src, 0, 0, "y"),
            Err(Refusal::WouldCapture("y".into()))
        );
        // Same scope: a duplicate definition (E0301) is refused too.
        assert!(matches!(
            edits("a = 1\nb = 2\nc: a\n", 0, 0, "b"),
            Err(Refusal::WouldCapture(_))
        ));
    }

    #[test]
    fn refuses_keywords_and_malformed_names() {
        let src = "x = 1\ny: x\n";
        assert!(matches!(edits(src, 0, 0, "end"), Err(Refusal::BadName(_))));
        assert!(matches!(edits(src, 0, 0, "2fa"), Err(Refusal::BadName(_))));
        assert!(matches!(edits(src, 0, 0, "a-b"), Err(Refusal::BadName(_))));
        assert!(matches!(edits(src, 0, 0, ""), Err(Refusal::BadName(_))));
    }

    #[test]
    fn refuses_pub_items_because_importers_are_invisible() {
        let src = "pub def make()\n  x: 1\nend\n";
        assert_eq!(
            edits(src, 0, 8, "build"),
            Err(Refusal::IsPublic("make".into()))
        );
        assert!(prepare(src, 0, 8).is_err(), "refused before prompting");
    }

    #[test]
    fn refuses_non_bindings() {
        // A stdlib method name and a schema field are not renameable symbols.
        let src = "xs = [1]\nys: xs.map (v, i) -> v end\ntype E\n  host: String\nend\n";
        assert_eq!(edits(src, 1, 8, "z"), Err(Refusal::NotABinding)); // on `map`
        assert_eq!(edits(src, 3, 3, "hostname"), Err(Refusal::NotABinding)); // on `host`
    }

    #[test]
    fn prepare_returns_the_declaration_range_and_renaming_to_itself_is_a_noop() {
        let src = "count = 1\ny: count\n";
        let r = prepare(src, 1, 4).unwrap();
        assert_eq!((r.start.line, r.start.character), (0, 0));
        assert_eq!(r.end.character, 5);
        assert!(edits(src, 0, 0, "count").unwrap().is_empty());
    }
}

/// The property that actually matters: a rename is a refactor, so the manifest's
/// output must be byte-identical afterwards. Anything else means the rename
/// changed what would be deployed.
///
/// This renames *every* binding in the real showcase manifest — one at a time,
/// each in a fresh copy — and compares the evaluated JSON against the original.
#[cfg(test)]
mod invariance {
    use std::path::{Path, PathBuf};

    use aura_lang::facade::{eval_file, EvalOptions};

    use super::tests_support::apply_at_offset;

    fn examples_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/showcase")
            .canonicalize()
            .expect("examples/showcase exists")
    }

    fn opts(dir: &Path) -> EvalOptions {
        EvalOptions {
            allow_read: vec![dir.to_path_buf()],
            allow_env: aura_lang::eval::EnvCap::Allow(vec!["APP_ENV".to_string()]),
            ..Default::default()
        }
    }

    #[test]
    fn renaming_any_binding_leaves_the_evaluated_json_identical() {
        let dir = examples_dir();
        let original = std::fs::read_to_string(dir.join("showcase.aura")).expect("read showcase");

        // The manifest imports ./lib.aura and reads ./data.json relative to the
        // process CWD, so the whole check runs inside one scratch copy: baseline
        // and renamed variants are then evaluated under identical conditions.
        let scratch = std::env::temp_dir().join("aura-rename-invariance");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("scratch dir");
        for f in ["lib.aura", "data.json"] {
            std::fs::copy(dir.join(f), scratch.join(f)).unwrap_or_else(|e| panic!("copy {f}: {e}"));
        }
        std::env::set_var("APP_ENV", "production");
        std::env::set_current_dir(&scratch).expect("chdir to scratch");

        let path = scratch.join("showcase.aura");
        std::fs::write(&path, &original).expect("write baseline");
        let baseline = eval_file(&path, &opts(&scratch))
            .expect("showcase evaluates")
            .json;

        let r = crate::goto::resolution(&original).expect("showcase parses");
        let mut renamed = 0usize;
        for (i, b) in r.bindings.iter().enumerate() {
            // A fresh, collision-free name for each binding.
            let new_name = format!("renamed_{i}");
            let Ok(text) = apply_at_offset(&original, b.decl.start, &new_name) else {
                continue; // refused (e.g. `pub`) — nothing to check
            };
            std::fs::write(&path, &text).expect("write copy");
            let after = eval_file(&path, &opts(&scratch)).unwrap_or_else(|e| {
                panic!(
                    "renaming '{}' -> '{new_name}' broke evaluation: {e:?}",
                    b.name
                )
            });
            assert_eq!(
                after.json, baseline,
                "renaming '{}' -> '{new_name}' changed the output",
                b.name
            );
            renamed += 1;
        }
        // Guard against the test silently checking nothing.
        assert!(
            renamed >= 15,
            "expected the showcase to exercise many bindings, got {renamed}"
        );
    }
}

/// Shared by the unit tests and the invariance test.
#[cfg(test)]
mod tests_support {
    use super::{edits, Refusal};
    use crate::diagnostics::LineIndex;

    /// Rename the binding whose declaration starts at `offset`, returning the new text.
    pub fn apply_at_offset(text: &str, offset: u32, new_name: &str) -> Result<String, Refusal> {
        let index = LineIndex::new(text);
        let pos = index.position(text, offset as usize);
        let ranges = edits(text, pos.line, pos.character, new_name)?;
        let mut spans: Vec<(usize, usize)> = ranges
            .iter()
            .map(|r| {
                (
                    index.offset(text, r.start.line, r.start.character),
                    index.offset(text, r.end.line, r.end.character),
                )
            })
            .collect();
        spans.sort_unstable();
        let mut out = text.to_string();
        for (s, e) in spans.into_iter().rev() {
            out.replace_range(s..e, new_name);
        }
        Ok(out)
    }
}
