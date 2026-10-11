//! The per-call budget (D33).
//!
//! A hermetic script can take nothing, but until this existed it could still
//! run forever: three nested `range(1000).map` — 95 bytes that pass
//! `check --hermetic` — ran for over a minute. A host calling a script per
//! request needs every call to end.
//!
//! Two counters, and neither is time. A time limit would make the outcome
//! depend on the machine: the same call could pass on a fast host and fail on
//! a slow one, which is the non-determinism this language exists to remove.
//!
//! - **Steps** — one per expression evaluated, counted by the interpreter.
//! - **Bytes** — the size of every value the call creates, and of every
//!   environment frame and binding, counted where they are built. Frames count
//!   because a call holds them until it returns; without them a lambda over a
//!   long list kept gigabytes alive while its value count stayed small. Cumulative rather than live: a value released halfway through
//!   still counts. That makes the number a property of the program and its
//!   input alone, and it bounds work as well as footprint — every builtin that
//!   loops, from `range` to `sort`, pays in values for what it produces.
//!
//! The byte counter is thread-local because a call runs on one thread and a
//! budget belongs to that call. Outside a budgeted call it starts at
//! `u64::MAX`, so the CLI and every existing caller see no limit.

use std::cell::Cell;
use std::mem::size_of;

use super::value::Value;

thread_local! {
    static BYTES_LEFT: Cell<u64> = const { Cell::new(u64::MAX) };
}

/// Charge `bytes` against the current call. Saturates at zero; the interpreter
/// notices on its next step.
#[inline]
pub(crate) fn charge(bytes: usize) {
    BYTES_LEFT.with(|c| c.set(c.get().saturating_sub(bytes as u64)));
}

/// One environment frame: the `Arc` header, the frame itself and the arena's
/// pointer to it. The binding map is charged per entry instead.
pub(crate) const ENV_COST: usize =
    size_of::<super::env::Environment<'_>>() + 2 * size_of::<usize>() + size_of::<usize>();

/// One binding in a frame, excluding the name's bytes, which are charged as
/// written: the map slot holding the key and the value.
pub(crate) const BINDING_COST: usize = size_of::<(String, Value<'_>)>() + size_of::<usize>();

#[inline]
pub(crate) fn charge_list(len: usize) {
    charge(len * size_of::<Value<'_>>());
}

#[inline]
pub(crate) fn bytes_exhausted() -> bool {
    BYTES_LEFT.with(|c| c.get() == 0)
}

/// Installs a byte budget for the lifetime of the guard and restores whatever
/// was there before, so a budgeted call nested inside another — or one that
/// returns early with an error — leaves the thread as it found it.
pub(crate) struct ByteBudget {
    previous: u64,
}

impl ByteBudget {
    pub(crate) fn install(bytes: u64) -> Self {
        let previous = BYTES_LEFT.with(|c| c.replace(bytes));
        ByteBudget { previous }
    }
}

impl Drop for ByteBudget {
    fn drop(&mut self) {
        BYTES_LEFT.with(|c| c.set(self.previous));
    }
}
