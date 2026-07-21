//! Lexical scope (SPEC §4.2, D9).
//!
//! D9 implementation: instead of a full chain of frozen prefixes, a frame uses
//! internal mutability (RefCell) while a block is being built. Values remain
//! immutable, redefinition within one frame is forbidden (E0301 in the interpreter),
//! and data references only point upward. A function's closure would close a
//! cycle back onto its defining env, so closures hold a `WeakEnv`; the
//! interpreter's env arena keeps every env alive for the eval instead. The observable
//! difference from strict freezing: a closure sees bindings from its own block
//! declared AFTER it, but it cannot be called before they are declared in the deterministic top-down execution order.

use indexmap::IndexMap;
use std::cell::RefCell;
use std::fmt;
use std::sync::{Arc, Weak};

use super::value::Value;

pub type Env<'a> = Arc<Environment<'a>>;

/// A non-owning reference to an environment. Used by function closures so that
/// the env->function->closure->env cycle does not leak: the interpreter's env
/// arena keeps every environment alive for the eval's duration instead (D9).
pub type WeakEnv<'a> = Weak<Environment<'a>>;

pub struct Environment<'a> {
    vars: RefCell<IndexMap<String, Value<'a>>>,
    parent: Option<Env<'a>>,
}

impl<'a> Environment<'a> {
    pub fn root() -> Env<'a> {
        Arc::new(Environment {
            vars: RefCell::new(IndexMap::new()),
            parent: None,
        })
    }

    pub fn child(parent: &Env<'a>) -> Env<'a> {
        Arc::new(Environment {
            vars: RefCell::new(IndexMap::new()),
            parent: Some(parent.clone()),
        })
    }

    /// Walks up the frame chain; cloning a value is O(1) (Arc).
    pub fn get(&self, name: &str) -> Option<Value<'a>> {
        if let Some(v) = self.vars.borrow().get(name) {
            return Some(v.clone());
        }
        self.parent.as_ref().and_then(|p| p.get(name))
    }

    pub fn defined_here(&self, name: &str) -> bool {
        self.vars.borrow().contains_key(name)
    }

    pub fn defined_in_ancestors(&self, name: &str) -> bool {
        match &self.parent {
            Some(p) => p.defined_here(name) || p.defined_in_ancestors(name),
            None => false,
        }
    }

    /// Low-level insertion; the E0301/E0302 rules are enforced by the interpreter.
    pub fn insert(&self, name: &str, v: Value<'a>) {
        self.vars.borrow_mut().insert(name.to_string(), v);
    }
}

impl fmt::Debug for Environment<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<env: {} vars>", self.vars.borrow().len())
    }
}
