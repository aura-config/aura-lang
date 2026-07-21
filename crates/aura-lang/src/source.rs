//! The source owner (SPEC §1.2): every `&'a str` in the lexer/AST/Value borrows from here.
//! An append-only arena: entries are never removed or mutated.

use std::cell::RefCell;

use crate::span::SourceId;

#[derive(Default)]
pub struct SourceCache {
    files: RefCell<Vec<(String, Box<str>)>>,
}

impl SourceCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a source and returns a slice with the cache's lifetime.
    pub fn add(&self, name: String, text: String) -> (SourceId, &str) {
        let boxed = text.into_boxed_str();
        let ptr: *const str = &*boxed;
        let mut files = self.files.borrow_mut();
        files.push((name, boxed));
        let id = (files.len() - 1) as SourceId;
        // SAFETY: the Box<str> data lives on the heap at a stable address; the Vec only moves
        // the Box itself (a pointer), entries in the append-only arena are never removed, so
        // the slice stays valid for as long as &self lives.
        (id, unsafe { &*ptr })
    }

    pub fn name(&self, id: SourceId) -> Option<String> {
        self.files.borrow().get(id as usize).map(|(n, _)| n.clone())
    }

    pub fn text(&self, id: SourceId) -> Option<&str> {
        let files = self.files.borrow();
        let (_, boxed) = files.get(id as usize)?;
        let ptr: *const str = &**boxed;
        // SAFETY: see `add`.
        Some(unsafe { &*ptr })
    }
}
