//! Stdlib surface for completion/hover, loaded by evaluating `stdlib.aura`.
//!
//! Dogfooding: the server runs aura-lang on its own Aura manifest to build the
//! completion database. A test (`manifest_matches_registry`) checks the manifest
//! against the real method registry so documentation and behaviour cannot drift.

use aura_lang::eval::{Interpreter, Options};
use aura_lang::lexer::Lexer;
use aura_lang::parser::Parser;
use aura_lang::serialize::to_json;

/// The manifest is embedded so the server is a single self-contained binary.
const MANIFEST: &str = include_str!("../stdlib.aura");

/// Aura keywords offered as completions: the lexer's reserved words (single
/// source of truth) plus the contextual block-string opener `text` (D16).
pub fn keywords() -> Vec<&'static str> {
    aura_lang::lexer::token::KEYWORDS
        .iter()
        .copied()
        .chain(std::iter::once("text"))
        .collect()
}

/// One method or builtin: `receiver` is a type name (`String`, `List`, …) or
/// `builtins` for a global function.
#[derive(Debug, Clone)]
pub struct Entry {
    pub receiver: String,
    pub name: String,
    pub params: Vec<String>,
    pub returns: String,
    pub doc: String,
}

impl Entry {
    /// A signature label such as `map(fn) -> List`.
    pub fn signature(&self) -> String {
        format!(
            "{}({}) -> {}",
            self.name,
            self.params.join(", "),
            self.returns
        )
    }
}

pub struct Stdlib {
    pub entries: Vec<Entry>,
}

impl Stdlib {
    /// Evaluate the embedded manifest. On any failure (a broken manifest, which
    /// the test catches) completion degrades to keywords only.
    pub fn load() -> Self {
        Stdlib {
            entries: eval_manifest(MANIFEST).unwrap_or_default(),
        }
    }

    pub fn methods(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.receiver != "builtins")
    }

    pub fn builtins(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.receiver == "builtins")
    }
}

fn eval_manifest(src: &str) -> Option<Vec<Entry>> {
    let tokens = Lexer::new(src, 0).tokenize().ok()?;
    let module = Parser::new(tokens).parse_module().ok()?;
    let mut interp = Interpreter::new(Options::default());
    let value = interp.eval_module(&module).ok()?;
    let json = to_json(&value).ok()?;

    let mut out = Vec::new();
    for (receiver, methods) in json.as_object()? {
        for (name, sig) in methods.as_object()? {
            let sig = sig.as_object()?;
            let params = sig
                .get("params")?
                .as_array()?
                .iter()
                .filter_map(|p| p.as_str().map(str::to_string))
                .collect();
            out.push(Entry {
                receiver: receiver.clone(),
                name: name.clone(),
                params,
                returns: sig.get("returns")?.as_str()?.to_string(),
                doc: sig.get("doc")?.as_str()?.to_string(),
            });
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aura_lang::eval::methods::MethodRegistry;
    use aura_lang::eval::value::TypeTag;
    use std::collections::HashSet;

    fn receiver_to_tag(name: &str) -> Option<TypeTag> {
        Some(match name {
            "String" => TypeTag::Str,
            "Int" => TypeTag::Int,
            "Float" => TypeTag::Float,
            "Bool" => TypeTag::Bool,
            "List" => TypeTag::List,
            "Object" => TypeTag::Object,
            _ => return None,
        })
    }

    #[test]
    fn manifest_loads_and_is_nonempty() {
        let sl = Stdlib::load();
        assert!(sl.methods().count() > 20, "manifest failed to evaluate");
        assert!(sl.builtins().any(|b| b.name == "range"));
    }

    /// Dogfooding guarantee: the Aura manifest and the Rust registry are the same
    /// set of `(receiver, method)` pairs — neither can drift without a red test.
    #[test]
    fn manifest_matches_registry() {
        let sl = Stdlib::load();
        let manifest: HashSet<(TypeTag, String)> = sl
            .methods()
            .filter_map(|e| receiver_to_tag(&e.receiver).map(|t| (t, e.name.clone())))
            .collect();
        let registry: HashSet<(TypeTag, String)> = MethodRegistry::builtin()
            .entries()
            .into_iter()
            .map(|(t, n)| (t, n.to_string()))
            .collect();

        let missing: Vec<_> = registry.difference(&manifest).collect();
        let extra: Vec<_> = manifest.difference(&registry).collect();
        assert!(
            missing.is_empty(),
            "registry methods absent from stdlib.aura: {missing:?}"
        );
        assert!(
            extra.is_empty(),
            "stdlib.aura methods not in the registry: {extra:?}"
        );
    }
}
