//! D36 §9: a reload frees the version it replaces.
//!
//! A service that reloads its rules on every deploy, or a watcher on a file a
//! tool rewrites often, swaps versions for the whole life of the process. If
//! each old version stayed alive - its sources, its environments, its exports -
//! memory would grow with every reload and never come back.
//!
//! Alone in its binary for the same reason as `script_memory.rs`: the counting
//! allocator is global and would count every other test running beside it.

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
fn memory_does_not_grow_with_the_number_of_reloads() {
    let dir = std::env::temp_dir().join(format!("aura-reload-mem-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join("r.aura");
    // Two versions that differ, so every reload is a real swap; each builds
    // lists and strings, so a leaked version cannot hide behind a tiny module.
    let version = |n: usize| {
        format!(
            "table: range(200).map (x, i) -> \"row-#{{x}}-{n}\" end
pub def f(x)
  v: x + {n}
end
"
        )
    };
    std::fs::write(&path, version(0)).expect("write");
    let s = Script::builder().file(&path).load().expect("loads");

    for i in 1..=50 {
        std::fs::write(&path, version(i % 2)).expect("write");
        s.reload().expect("reloads");
    }
    let before = LIVE.load(Relaxed);

    for i in 0..1_000 {
        std::fs::write(&path, version(i % 2)).expect("write");
        s.reload().expect("reloads");
        let _: serde_json::Value = s.call("f", (1,)).expect("call");
    }
    let grew = LIVE.load(Relaxed) - before;

    assert!(
        grew < 64 * 1024,
        "live memory grew by {grew} bytes over 1,000 reloads - {} bytes per reload",
        grew / 1_000
    );
}
