//! Reloading a script, as D36 promises it.
//!
//! Written before the implementation, against the designed API, one test per
//! decision. Security first, as the decision ranks it: a reload is a new load,
//! held to every check the first one passed, and it never gets more than the
//! first load was given. A test weakened to make an implementation pass is a
//! clause of D36 quietly withdrawn.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aura_lang::{Budget, Error, Script, ScriptVersion};
use serde::Deserialize;

#[derive(Deserialize, Debug, PartialEq)]
struct V {
    v: i64,
}

/// A fresh directory per test, so tests running in parallel never share files.
fn dir(test: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aura-reload-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("scratch dir");
    d
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, text).expect("write");
    p
}

fn returns(n: i64) -> String {
    format!("pub def f()\n  v: {n}\nend\n")
}

fn f(s: &Script) -> i64 {
    s.call::<_, V>("f", ()).expect("call").v
}

fn code(e: &Error) -> &'static str {
    e.code()
}

// ===========================================================================
// The ordinary case
// ===========================================================================

/// D36 §8. A reload picks up the new content and names the new version.
#[test]
fn a_reload_picks_up_the_new_content() {
    let d = dir("new-content");
    let path = write(&d, "r.aura", &returns(1));
    let s = Script::builder().file(&path).load().expect("loads");
    let first = s.version();
    assert_eq!(f(&s), 1);

    write(&d, "r.aura", &returns(2));
    let second = s.reload().expect("reloads");
    assert_eq!(f(&s), 2);
    assert_eq!(second.generation, first.generation + 1);
    assert_ne!(second.hash, first.hash);
    assert_eq!(s.version(), second);
}

/// D36 §1. Content decides, not the file's timestamp: rewriting the same bytes
/// is not a new version.
#[test]
fn the_same_content_is_not_a_new_version() {
    let d = dir("same-content");
    let path = write(&d, "r.aura", &returns(1));
    let s = Script::builder().file(&path).load().expect("loads");
    let first = s.version();

    write(&d, "r.aura", &returns(1));
    assert_eq!(s.reload().expect("reloads"), first);
    assert_eq!(s.version(), first);
}

/// D36 §5. An imported file is part of the script.
#[test]
fn a_change_in_an_import_is_a_change_in_the_script() {
    let d = dir("import");
    write(&d, "lib.aura", "n: 1\n");
    let path = write(
        &d,
        "r.aura",
        "import \"./lib.aura\" as lib\npub def f()\n  v: lib.n\nend\n",
    );
    let s = Script::builder().file(&path).load().expect("loads");
    assert_eq!(f(&s), 1);

    write(&d, "lib.aura", "n: 2\n");
    s.reload().expect("reloads");
    assert_eq!(f(&s), 2);
}

// ===========================================================================
// 1. Security: a reload gets nothing the first load did not
// ===========================================================================

/// D36 §3. A reload is analysed like a load: a hermetic script cannot gain
/// `env()` by being edited.
#[test]
fn a_reload_cannot_become_less_hermetic() {
    let d = dir("hermetic");
    let path = write(&d, "r.aura", &returns(1));
    let s = Script::builder().file(&path).load().expect("loads");

    write(
        &d,
        "r.aura",
        "pub def f()\n  v: 2\n  h: env(\"HOME\", \"-\")\nend\n",
    );
    let err = s.reload().unwrap_err();
    assert_eq!(code(&err), "E0505", "{err}");
    assert_eq!(f(&s), 1, "the old version keeps serving");
}

/// D36 §3. Host functions are checked again: a new version cannot import one
/// the host did not provide.
#[test]
fn a_reload_cannot_reach_an_unprovided_host_function() {
    let d = dir("host");
    let path = write(&d, "r.aura", &returns(1));
    let s = Script::builder()
        .file(&path)
        .pure("geo.region", |_ip: String| "eu".to_string())
        .load()
        .expect("loads");

    write(
        &d,
        "r.aura",
        "import \"host:db\" as db\npub def f()\n  v: db.user(1)\nend\n",
    );
    let err = s.reload().unwrap_err();
    assert_eq!(code(&err), "E0007", "{err}");
    assert_eq!(f(&s), 1);
}

/// D36 §4. The lock is read once. A reload resolves as `--frozen` does, so a
/// new version cannot pull in a package the first load did not pin.
#[test]
fn a_reload_cannot_add_an_unpinned_package() {
    let d = dir("frozen");
    let path = write(&d, "r.aura", &returns(1));
    let s = Script::builder().file(&path).load().expect("loads");

    write(
        &d,
        "r.aura",
        "import github/acme/pkg@v1.0.0 as pkg\npub def f()\n  v: 2\nend\n",
    );
    let err = s.reload().unwrap_err();
    assert_eq!(code(&err), "E0403", "{err}");
    assert_eq!(f(&s), 1);
}

/// D36 §10. A reload is budgeted: a module that never finishes is stopped, and
/// the old version stays.
#[test]
fn a_reload_is_held_to_the_budget() {
    let d = dir("budget");
    let path = write(&d, "r.aura", &returns(1));
    let s = Script::builder()
        .file(&path)
        .budget(Budget::per_call().steps(100_000).memory_mb(64))
        .load()
        .expect("loads");

    write(
        &d,
        "r.aura",
        "n: range(1000000).map (x, i) -> x end.len()\npub def f()\n  v: 2\nend\n",
    );
    let err = s.reload().unwrap_err();
    assert_eq!(code(&err), "E0325", "{err}");
    assert_eq!(f(&s), 1);
}

// ===========================================================================
// 2. Integrity: the contract the host bound to
// ===========================================================================

/// D36 §2. Removing an export the host may have bound is refused.
#[test]
fn removing_an_export_is_e0011() {
    let d = dir("remove");
    let path = write(
        &d,
        "r.aura",
        &format!("{}pub def g()\n  v: 0\nend\n", returns(1)),
    );
    let s = Script::builder().file(&path).load().expect("loads");

    write(&d, "r.aura", &returns(2));
    let err = s.reload().unwrap_err();
    assert_eq!(code(&err), "E0011", "{err}");
    assert!(err.to_string().contains('g'), "{err}");
    assert_eq!(f(&s), 1);
}

/// D36 §2. Changing a function's number of arguments is refused.
#[test]
fn changing_the_arity_is_e0011() {
    let d = dir("arity");
    let path = write(&d, "r.aura", &returns(1));
    let s = Script::builder().file(&path).load().expect("loads");

    write(&d, "r.aura", "pub def f(x)\n  v: x\nend\n");
    let err = s.reload().unwrap_err();
    assert_eq!(code(&err), "E0011", "{err}");
    assert_eq!(f(&s), 1);
}

/// D36 §2. Turning a function into a value is refused.
#[test]
fn changing_the_kind_is_e0011() {
    let d = dir("kind");
    let path = write(&d, "r.aura", &returns(1));
    let s = Script::builder().file(&path).load().expect("loads");

    write(&d, "r.aura", "f: 1\n");
    let err = s.reload().unwrap_err();
    assert_eq!(code(&err), "E0011", "{err}");
    assert_eq!(f(&s), 1);
}

/// D36 §2. Adding an export is allowed: nothing the host bound can break.
#[test]
fn adding_an_export_is_allowed() {
    let d = dir("add");
    let path = write(&d, "r.aura", &returns(1));
    let s = Script::builder().file(&path).load().expect("loads");

    write(
        &d,
        "r.aura",
        &format!("{}pub def g()\n  v: 7\nend\n", returns(2)),
    );
    s.reload().expect("reloads");
    assert_eq!(s.call::<_, V>("g", ()).expect("call").v, 7);
}

/// D36 §2. A bound function finds its export by name. A new version that
/// lists its exports in another order must not make it call another function.
#[test]
fn a_bound_function_follows_its_name_not_its_position() {
    let d = dir("order");
    let path = write(
        &d,
        "r.aura",
        "pub def f()\n  v: 1\nend\npub def g()\n  v: 100\nend\n",
    );
    let s = Script::builder().file(&path).load().expect("loads");
    let bound = s.function::<(), V>("f").expect("binds");
    assert_eq!(bound.call(()).expect("call").v, 1);

    write(
        &d,
        "r.aura",
        "pub def g()\n  v: 200\nend\npub def f()\n  v: 2\nend\n",
    );
    s.reload().expect("reloads");
    assert_eq!(bound.call(()).expect("call").v, 2, "called g, not f");
}

/// D36 §9. A reader sees the old version or the new one, never half: every
/// call returns two fields from the same version while versions are swapped
/// under it from another thread.
#[test]
fn concurrent_calls_never_see_half_a_version() {
    let d = dir("concurrent");
    let version = |n: i64| format!("pub def f()\n  a: {n}\n  b: {n}\nend\n");
    let path = write(&d, "r.aura", &version(0));
    let s = Arc::new(Script::builder().file(&path).load().expect("loads"));

    #[derive(Deserialize)]
    struct Pair {
        a: i64,
        b: i64,
    }
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let s = s.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut seen = 0;
                while !stop.load(Ordering::Relaxed) {
                    let p: Pair = s.call("f", ()).expect("call");
                    assert_eq!(p.a, p.b, "a call saw two versions at once");
                    seen += 1;
                }
                seen
            })
        })
        .collect();
    for n in 1..=50 {
        write(&d, "r.aura", &version(n));
        s.reload().expect("reloads");
    }
    stop.store(true, Ordering::Relaxed);
    for r in readers {
        assert!(r.join().expect("reader") > 0);
    }
    assert_eq!(s.version().generation, 51);
}

/// D36 §7. A script held in memory has no files to read again.
#[test]
fn an_in_memory_script_is_not_reloaded() {
    let s = Script::builder()
        .source("r.aura", returns(1))
        .load()
        .expect("loads");
    let err = s.reload().unwrap_err();
    assert_eq!(code(&err), "E0001", "{err}");
}

// ===========================================================================
// 3. Watching
// ===========================================================================

const TICK: Duration = Duration::from_millis(20);

/// The interval for the tests where a writer races the watcher. A writer that
/// stalls between truncating a file and filling it, for longer than two
/// intervals, has left a file that really is empty and unchanged - no watcher
/// can tell that from one emptied on purpose, which is why D36 tells writers to
/// publish by rename. With 20 ms, a loaded CI runner stalled the writer that
/// long and these tests failed for the scheduler's reason, not the watcher's;
/// reproduced on Linux under load. At 250 ms the stall would have to pass half
/// a second.
const WRITING_TICK: Duration = Duration::from_millis(250);

/// Every result the watcher reported, in order.
type Reports = Arc<Mutex<Vec<Result<ScriptVersion, String>>>>;

fn watched(path: &Path) -> (Script, Reports) {
    watched_every(path, TICK)
}

fn watched_every(path: &Path, interval: Duration) -> (Script, Reports) {
    let reports: Reports = Arc::default();
    let sink = reports.clone();
    let s = Script::builder()
        .file(path)
        .watch(interval, move |r: Result<ScriptVersion, Error>| {
            sink.lock()
                .expect("reports")
                .push(r.map_err(|e| e.code().to_string()));
        })
        .load()
        .expect("loads");
    (s, reports)
}

/// Waits for `cond`, up to a bound generous enough for a loaded CI machine.
fn eventually(mut cond: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if cond() {
            return true;
        }
        std::thread::sleep(TICK);
    }
    false
}

/// D36 §1. A watched script picks up a change by itself and reports it.
#[test]
fn a_watched_script_reloads_on_a_change() {
    let d = dir("watch-change");
    let path = write(&d, "r.aura", &returns(1));
    let (s, reports) = watched(&path);

    write(&d, "r.aura", &returns(2));
    assert!(eventually(|| f(&s) == 2), "the change was never picked up");
    let r = reports.lock().expect("reports");
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!(r[0].as_ref().expect("ok").generation, 2);
}

/// D36 §1. Touching a file without changing it is not a change.
#[test]
fn a_touch_is_not_a_change() {
    let d = dir("watch-touch");
    let path = write(&d, "r.aura", &returns(1));
    let (s, reports) = watched(&path);

    for _ in 0..5 {
        write(&d, "r.aura", &returns(1));
        std::thread::sleep(TICK * 3);
    }
    assert!(reports.lock().expect("reports").is_empty());
    assert_eq!(s.version().generation, 1);
}

/// D36 §1. A touch does not even cost a load. The module calls a host function
/// while it loads, so the count of its calls is the count of loads.
#[test]
fn a_touch_does_not_load_the_script_again() {
    let d = dir("watch-touch-cost");
    let path = write(&d, "r.aura", "import \"host:k\" as k\nn: k.load()\n");
    let loads = Arc::new(AtomicUsize::new(0));
    let l = loads.clone();
    let _s = Script::builder()
        .file(&path)
        .pure("k.load", move || l.fetch_add(1, Ordering::Relaxed) as i64)
        .watch(TICK, |_| {})
        .load()
        .expect("loads");
    assert_eq!(loads.load(Ordering::Relaxed), 1);

    for _ in 0..5 {
        write(&d, "r.aura", "import \"host:k\" as k\nn: k.load()\n");
        std::thread::sleep(TICK * 3);
    }
    assert_eq!(
        loads.load(Ordering::Relaxed),
        1,
        "a touch loaded the script"
    );
}

/// D36 §6. Content that does not hold still costs no load. The file is changed
/// for a quarter of an interval and put back, again and again, each time to
/// different text: two reads one interval apart never see the same change, so
/// none is loaded - not even tried. Found on macOS CI, where a rewrite of the
/// same bytes was read half done and loaded once for nothing; Windows never
/// showed it, which is why this test makes the brief change itself instead of
/// hoping to catch one. Each change is unique because the first version of
/// this test reused one text, and on a slow runner the second read landed in
/// the next change, saw the same bytes and called them stable.
#[test]
fn a_change_that_does_not_hold_still_is_never_loaded() {
    let d = dir("watch-blip");
    let body = "import \"host:k\" as k\nn: k.load()\n";
    let path = write(&d, "r.aura", body);
    let loads = Arc::new(AtomicUsize::new(0));
    let l = loads.clone();
    let interval = Duration::from_millis(200);
    let _s = Script::builder()
        .file(&path)
        .pure("k.load", move || l.fetch_add(1, Ordering::Relaxed) as i64)
        .watch(interval, |_| {})
        .load()
        .expect("loads");

    for i in 0..15 {
        write(&d, "r.aura", &format!("{body}# passing edit {i}\n"));
        std::thread::sleep(interval / 4);
        write(&d, "r.aura", body);
        std::thread::sleep(interval + interval / 2);
    }
    assert_eq!(
        loads.load(Ordering::Relaxed),
        1,
        "content that never held still for an interval was loaded"
    );
}

/// D36 §1, §9. A dropped script is freed, watcher and all: nothing it holds -
/// here, a host function's captured state - outlives it.
#[test]
fn dropping_a_watched_script_frees_it() {
    let d = dir("watch-free");
    let path = write(&d, "r.aura", &returns(1));
    let held = Arc::new(());
    let h = held.clone();
    let s = Script::builder()
        .file(&path)
        .pure("k.held", move || Arc::strong_count(&h) as i64)
        .watch(TICK, |_| {})
        .load()
        .expect("loads");
    assert_eq!(Arc::strong_count(&held), 2);
    drop(s);
    assert_eq!(
        Arc::strong_count(&held),
        1,
        "the dropped script is still alive"
    );
}

/// D36 §6. A file that is still being written is not loaded. A writer rewrites
/// the file every few milliseconds, far faster than the interval, so no two
/// reads one interval apart agree until it stops - and only then is anything
/// loaded: the final content, once.
#[test]
fn a_file_still_being_written_is_not_loaded() {
    let d = dir("watch-writing");
    let path = write(&d, "r.aura", &returns(1));
    let (s, reports) = watched_every(&path, WRITING_TICK);

    let end = Instant::now() + WRITING_TICK * 6;
    let mut n = 2;
    while Instant::now() < end {
        write(&d, "r.aura", &returns(n));
        n += 1;
        std::thread::sleep(Duration::from_millis(2));
    }
    write(&d, "r.aura", &returns(999));
    assert!(
        eventually(|| f(&s) == 999),
        "the final content never loaded"
    );
    let r = reports.lock().expect("reports");
    assert_eq!(
        r.len(),
        1,
        "loaded a version while it was being written: {r:?}"
    );
}

/// D36 §6. Same bytes are not enough: a file someone is still writing is not
/// stable even when two reads agree. A writer here rewrites the file with the
/// same new text every 2 ms, so every read sees the same bytes - and the file
/// is never at rest. Nothing may load until the writer stops. Found on Ubuntu
/// CI, where a writer's truncations were read empty twice, an interval apart,
/// and an empty script was loaded (E0011: `f` gone).
#[test]
fn a_file_being_rewritten_is_not_loaded_even_if_it_reads_the_same() {
    let d = dir("watch-rewriting");
    let path = write(&d, "r.aura", &returns(1));
    let (s, reports) = watched_every(&path, WRITING_TICK);

    let end = Instant::now() + WRITING_TICK * 6;
    while Instant::now() < end {
        write(&d, "r.aura", &returns(2));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        reports.lock().expect("reports").is_empty(),
        "loaded while the file was still being written: {:?}",
        reports.lock().expect("reports")
    );
    assert!(eventually(|| f(&s) == 2), "the final content never loaded");
    assert_eq!(reports.lock().expect("reports").len(), 1);
}

/// D36 §12. A failed automatic reload is reported - once for the same content,
/// not on every tick - and the old version keeps serving.
#[test]
fn a_failed_watch_reload_is_reported_once() {
    let d = dir("watch-fail");
    let path = write(&d, "r.aura", &returns(1));
    let (s, reports) = watched(&path);

    write(&d, "r.aura", "pub def f(\n");
    assert!(eventually(|| !reports.lock().expect("reports").is_empty()));
    std::thread::sleep(TICK * 10);
    let r = reports.lock().expect("reports");
    assert_eq!(r.len(), 1, "reported more than once: {r:?}");
    assert!(r[0].is_err());
    assert_eq!(f(&s), 1);
}

/// D36 §1. The watcher belongs to the script: once the script is dropped,
/// nothing is watched and the handler is never called again.
#[test]
fn dropping_the_script_stops_the_watcher() {
    let d = dir("watch-drop");
    let path = write(&d, "r.aura", &returns(1));
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let s = Script::builder()
        .file(&path)
        .watch(TICK, move |_| {
            c.fetch_add(1, Ordering::Relaxed);
        })
        .load()
        .expect("loads");
    drop(s);

    write(&d, "r.aura", &returns(2));
    std::thread::sleep(TICK * 10);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

/// D36 §7. Watching needs files.
#[test]
fn an_in_memory_script_cannot_be_watched() {
    let err = Script::builder()
        .source("r.aura", returns(1))
        .watch(TICK, |_| {})
        .load()
        .unwrap_err();
    assert_eq!(code(&err), "E0001", "{err}");
}
