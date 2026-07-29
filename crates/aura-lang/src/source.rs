//! The source owner (SPEC §1.2): every `&'a str` in the lexer/AST/Value borrows from here.
//! An append-only arena: entries are never removed or mutated.

use std::cell::RefCell;

use crate::span::SourceId;

/// Every source text, kept alive as long as the cache is, so the zero-copy
/// pipeline can hand out `&str` slices into it.
///
/// The texts are held as raw pointers from `Box::into_raw`, which is load-bearing
/// and was arrived at the hard way — Miri rejected the two obvious alternatives:
///
/// * Storing `Box<str>` and taking a reference before pushing. `Vec::push` *moves*
///   the box, and moving one re-asserts unique ownership of its contents, which
///   invalidates every slice already handed out. Reallocation moves them again.
///   `Box` carries a genuine `noalias` guarantee that LLVM exploits, so this is
///   not a technicality.
/// * Storing `&'static str` from `Box::leak`. Handing out slices is then safe, but
///   `Drop` has to free them, and reconstructing a `Box` from a *shared* reference
///   deallocates through provenance that never granted write access.
///
/// A raw pointer has no aliasing guarantee to violate: moving it inside the `Vec`
/// retags nothing, shared references derived from it stay valid, and `Drop` frees
/// through the same provenance it was created with. Checked under both Stacked
/// Borrows and Tree Borrows; see `references_survive_later_insertions_and_reallocation`.
#[derive(Default)]
pub struct SourceCache {
    files: RefCell<Vec<(String, *mut str)>>,
}

impl SourceCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a source and returns a slice with the cache's lifetime.
    ///
    /// The slice is returned bounded by `&self`, so no caller can outlive the
    /// arena that owns it.
    pub fn add(&self, name: String, text: String) -> (SourceId, &str) {
        let ptr: *mut str = Box::into_raw(text.into_boxed_str());
        let mut files = self.files.borrow_mut();
        files.push((name, ptr));
        let id = (files.len() - 1) as SourceId;
        // SAFETY: `ptr` owns a live allocation from `Box::into_raw`. Nothing moves
        // or frees the string data before `Drop`, and the arena never removes or
        // mutates an entry, so the slice is valid for as long as `&self`.
        (id, unsafe { &*ptr })
    }

    pub fn name(&self, id: SourceId) -> Option<String> {
        self.files.borrow().get(id as usize).map(|(n, _)| n.clone())
    }

    pub fn text(&self, id: SourceId) -> Option<&str> {
        // The pointer is copied out before the borrow ends, so nothing borrowed
        // from the `RefCell` escapes it.
        let ptr = *self.files.borrow().get(id as usize).map(|(_, p)| p)?;
        // SAFETY: see `add`.
        Some(unsafe { &*ptr })
    }
}

impl Drop for SourceCache {
    fn drop(&mut self) {
        for (_, ptr) in self.files.borrow_mut().drain(..) {
            // SAFETY: each pointer came from `Box::into_raw` in `add`, is owned by
            // this arena alone, and is reconstructed exactly once. Every slice
            // handed out was bounded by `&self`, so none outlives this.
            unsafe { drop(Box::from_raw(ptr)) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The arena hands out references that outlive the `RefCell` borrow they were
    /// derived from, which is the whole point and also the only reason this file
    /// contains `unsafe`. Run under Miri (`cargo +nightly miri test -p aura-lang
    /// source::`) to check that against the aliasing model rather than against a
    /// comment.
    #[test]
    fn references_survive_later_insertions_and_reallocation() {
        let cache = SourceCache::new();
        let (first_id, first) = cache.add(
            "a.aura".into(),
            "port: 8080
"
            .into(),
        );

        // Enough pushes to force the Vec to reallocate several times. The Box
        // contents must not move with it.
        let mut held = vec![first];
        for i in 0..64 {
            let (_, text) = cache.add(
                format!("f{i}.aura"),
                format!(
                    "x: {i}
"
                ),
            );
            held.push(text);
        }

        // The first slice is still readable, and still says what it said.
        assert_eq!(
            first,
            "port: 8080
"
        );
        assert_eq!(
            cache.text(first_id),
            Some(
                "port: 8080
"
            )
        );
        for (i, text) in held.iter().skip(1).enumerate() {
            assert_eq!(
                *text,
                format!(
                    "x: {i}
"
                )
            );
        }

        // A slice fetched through `text()` outlives the `Ref` guard inside it.
        let borrowed = cache.text(first_id).unwrap();
        let _ = cache.add(
            "later.aura".into(),
            "y: 1
"
            .into(),
        );
        assert_eq!(
            borrowed,
            "port: 8080
"
        );
    }

    #[test]
    fn an_unknown_id_is_none_rather_than_a_panic() {
        let cache = SourceCache::new();
        assert_eq!(cache.text(7), None);
        assert_eq!(cache.name(7), None);
    }
}
