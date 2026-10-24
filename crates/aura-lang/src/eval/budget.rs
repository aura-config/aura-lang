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
    /// D43: a container deeper than `MAX_VALUE_DEPTH` was made. Set where the
    /// container is built, read at the next step - as the byte budget is - so
    /// no constructor has to return a `Result`.
    /// Holds the byte budget it set aside, to give back once the error is out.
    static TOO_DEEP: Cell<Option<u64>> = const { Cell::new(None) };
}

/// D43: stop the evaluation at its next step. The byte budget is emptied too,
/// so `step` keeps the one check it had - this is the rare path, and the
/// common one pays nothing for it - and tells the two causes apart only once
/// that check has fired.
#[cold]
pub(crate) fn value_too_deep() {
    TOO_DEEP.with(|deep| {
        if deep.get().is_none() {
            deep.set(Some(BYTES_LEFT.with(|c| c.replace(0))));
        }
    });
}

/// Whether the stop was a too-deep value rather than spent memory. If so, the
/// byte budget it set aside is restored, so a thread with no budget installed
/// - an interpreter driven directly - is not left believing memory ran out.
pub(crate) fn take_too_deep() -> bool {
    match TOO_DEEP.with(|c| c.take()) {
        Some(bytes) => {
            BYTES_LEFT.with(|c| c.set(bytes));
            true
        }
        None => false,
    }
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

/// What is left of this thread's byte budget, so evaluation moved to another
/// thread (D42, debug builds) can carry it there and bring it back.
#[cfg(all(not(target_family = "wasm"), debug_assertions))]
pub(crate) fn bytes_left() -> u64 {
    BYTES_LEFT.with(Cell::get)
}

#[cfg(all(not(target_family = "wasm"), debug_assertions))]
pub(crate) fn set_bytes_left(bytes: u64) {
    BYTES_LEFT.with(|c| c.set(bytes));
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
        // A too-deep value from an earlier evaluation on this thread is not
        // this one's error (D43).
        TOO_DEEP.with(|c| c.set(None));
        ByteBudget { previous }
    }
}

impl Drop for ByteBudget {
    fn drop(&mut self) {
        BYTES_LEFT.with(|c| c.set(self.previous));
    }
}

/// How much one call may do (D33). Counted in steps and bytes, never in time:
/// a time limit would let the same call pass on a fast machine and fail on a
/// slow one.
///
/// The default is finite. A host that sets nothing is protected all the same.
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    steps: u64,
    bytes: u64,
}

impl Budget {
    /// Ten million expressions and 256 MiB of values per call. Provisional:
    /// D33 leaves the final numbers to the benchmarks.
    pub const DEFAULT: Budget = Budget {
        steps: 10_000_000,
        bytes: 256 * 1024 * 1024,
    };

    /// The default budget, to be narrowed or widened.
    pub fn per_call() -> Self {
        Self::DEFAULT
    }

    /// Expressions one call may evaluate.
    pub fn steps(mut self, steps: u64) -> Self {
        self.steps = steps;
        self
    }

    /// Bytes of values one call may create — cumulatively, released or not, so
    /// the count is a property of the program and its input alone.
    pub fn memory_bytes(mut self, bytes: u64) -> Self {
        self.bytes = bytes;
        self
    }

    /// [`Budget::memory_bytes`] in mebibytes.
    pub fn memory_mb(self, mb: u64) -> Self {
        self.memory_bytes(mb.saturating_mul(1024 * 1024))
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What an evaluation actually spent, so a host — or someone who just hit
/// `E0325` — can see how much budget a manifest needs instead of guessing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub steps: u64,
    pub bytes: u64,
}

/// A budget in force on one interpreter, for as long as this lives: the step
/// count set on the interpreter, the byte count installed for the thread. Every
/// path that evaluates — the CLI, the facade, `Script` — starts one the same
/// way, so none of them can apply half a budget.
pub struct Metered {
    _bytes: ByteBudget,
    budget: Budget,
}

impl Budget {
    /// Put this budget in force on `interp` until the returned guard is dropped.
    pub fn begin(self, interp: &mut super::Interpreter<'_>) -> Metered {
        interp.set_step_budget(self.steps);
        Metered {
            _bytes: ByteBudget::install(self.bytes),
            budget: self,
        }
    }
}

impl Metered {
    /// What has been spent since `begin`.
    pub fn usage(&self, interp: &super::Interpreter<'_>) -> Usage {
        Usage {
            steps: self.budget.steps - interp.steps_left(),
            bytes: self.budget.bytes - BYTES_LEFT.with(Cell::get),
        }
    }
}
