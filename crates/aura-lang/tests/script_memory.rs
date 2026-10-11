//! D33 §6: memory per call is constant.
//!
//! The route this API replaces kept every call's environment in the
//! interpreter's arena for the interpreter's whole life. Measured before D33:
//! about 2 KB per call, linearly, so a million calls held 2 GB. A service that
//! calls a rule per request — or a telemetry pipeline at a million events a
//! second — would run out of memory in minutes.
//!
//! This binary installs a counting global allocator, which is why the test is
//! alone in it: a global allocator is per binary, and it would otherwise count
//! every other test running beside it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};

use aura_lang::Script;

struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

// SAFETY: forwards every call to the system allocator unchanged; the counter
// is the only addition, and it is an atomic touched by nothing else.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Relaxed);
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn memory_does_not_grow_with_the_number_of_calls() {
    // A function that allocates on every call — a lambda environment per
    // element, a new list, interpolated strings — so a leak cannot hide behind
    // a body that allocates nothing.
    let rules = Script::builder()
        .source(
            "t.aura",
            concat!(
                "pub def route(path)\n",
                "  backend: path.starts_with(\"/api\") ? \"api\" : \"static\"\n",
                "  tags:    [1, 2, 3, 4, 5].map (x, i) -> \"#{path}-#{x}\" end\n",
                "end\n",
            ),
        )
        .load()
        .expect("loads");
    let route = rules
        .function::<(&str,), serde_json::Value>("route")
        .expect("binds");

    // Warm up: first calls may populate caches, and that is allowed — what is
    // not allowed is growth that continues with the number of calls.
    for _ in 0..1_000 {
        route.call(("/api/x",)).expect("call");
    }
    let before = LIVE.load(Relaxed);

    for _ in 0..100_000 {
        route.call(("/api/x",)).expect("call");
    }
    let grew = LIVE.load(Relaxed) - before;

    // 100,000 calls on the old route would have kept about 200 MB.
    assert!(
        grew < 64 * 1024,
        "live memory grew by {grew} bytes over 100,000 calls — {} bytes per call",
        grew / 100_000
    );
}
