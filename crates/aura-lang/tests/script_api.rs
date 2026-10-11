//! The embedding API, as D33 promises it.
//!
//! These tests were written before the implementation, against the API as it
//! was designed, and each one is a clause of D33. They are grouped the way the
//! decision ranks its requirements: security first, then resources, then the
//! boundary with the host. A test here that is weakened to make an
//! implementation pass is a clause of the decision quietly withdrawn.
//!
//! Two of them encode defects measured on the old route before this existed:
//! a call that left ~2 KB in an arena forever, and a 95-byte manifest that
//! passed `check --hermetic` and ran for over a minute.

use std::time::{Duration, Instant};

use aura_lang::{Budget, Error, Script};
use serde::{Deserialize, Serialize};

fn load(src: &str) -> Script {
    Script::builder()
        .source("t.aura", src)
        .load()
        .unwrap_or_else(|e| panic!("load failed:\n{e}"))
}

fn load_err(src: &str) -> Error {
    match Script::builder().source("t.aura", src).load() {
        Ok(_) => panic!("load was expected to fail"),
        Err(e) => e,
    }
}

fn code(e: &Error) -> &'static str {
    e.code()
}

// ===========================================================================
// 1. Security
// ===========================================================================

/// D33 §1. With no grants, an effect anywhere fails the *load* — including on a
/// branch that would never run, which is what makes it a proof rather than an
/// observation of one run.
#[test]
fn load_is_hermetic_by_default() {
    let e = load_err(concat!(
        "pub def f(x)\n",
        "  v: x > 1000 ? env(\"HOME\", \"-\") : \"never\"\n",
        "end\n",
    ));
    assert_eq!(code(&e), "E0505", "{e}");
}

/// D33 §2. A grant has the name and meaning of the CLI flag, and covers exactly
/// what it names.
#[test]
fn grants_mirror_the_cli() {
    let rules = Script::builder()
        .source(
            "t.aura",
            "pub def f()\n  r: env(\"AURA_TEST_REGION\", \"none\")\nend\n",
        )
        .allow_env(["AURA_TEST_REGION"])
        .load()
        .expect("granted variable loads");
    let _: serde_json::Value = rules.call("f", ()).expect("granted variable is readable");

    let denied = Script::builder()
        .source("t.aura", "pub def f()\n  h: env(\"HOME\", \"none\")\nend\n")
        .allow_env(["AURA_TEST_REGION"])
        .load()
        .expect("loads: the analysis cannot know which name is read")
        .call::<_, serde_json::Value>("f", ());
    assert_eq!(code(&denied.unwrap_err()), "E0310");
}

/// D33 §2, D1, D12. A grant reaches the root module only.
#[test]
fn grants_do_not_reach_imports() {
    let rules = Script::builder()
        .source(
            "lib.aura",
            "pub def peek()\n  v: env(\"AURA_TEST_REGION\", \"-\")\nend\n",
        )
        .source(
            "main.aura",
            "import \"./lib.aura\" as lib\npub def f()\n  v: lib.peek()\nend\n",
        )
        .entry("main.aura")
        .allow_env(["AURA_TEST_REGION"])
        .load()
        .expect("loads");
    let err = rules.call::<_, serde_json::Value>("f", ()).unwrap_err();
    assert_eq!(code(&err), "E0310", "{err}");
}

/// D33 §3. The manifest that motivated the budget: three nested maps, 95 bytes,
/// hermetic, and over a minute of work. It must be stopped by a budget — and
/// stopped soon, which is the point of having one.
///
/// Which counter trips first is not the clause: this program builds a billion
/// values, so the memory budget usually wins the race. The step counter on its
/// own is tested by the next test, with a program that allocates nothing.
#[test]
fn a_budget_stops_the_runaway_that_motivated_it() {
    let rules = load(concat!(
        "pub def f()\n",
        "  x: range(1000).map (a, i) -> range(1000).map (b, j) -> ",
        "range(1000).map (c, k) -> 1 end end end\n",
        "end\n",
    ));
    let started = Instant::now();
    let err = rules.call::<_, serde_json::Value>("f", ()).unwrap_err();
    assert!(matches!(code(&err), "E0325" | "E0326"), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the budget did not stop the call promptly: {:?}",
        started.elapsed()
    );
}

/// D33 §3. The step counter alone. Recursion through a function allocates no
/// list at each level, so only steps can stop it — and the memory budget is set
/// out of reach to make sure nothing else does.
#[test]
fn the_step_budget_stops_a_call_that_allocates_nothing() {
    let rules = Script::builder()
        .source(
            "t.aura",
            concat!(
                "pub def f()\n",
                "  n: range(200).reduce(0) (acc, a) -> range(200).reduce(acc) (acc2, b) -> ",
                "range(200).reduce(acc2) (acc3, c) -> acc3 + 1 end end end\n",
                "end\n",
            ),
        )
        .budget(Budget::per_call().steps(100_000).memory_bytes(u64::MAX))
        .load()
        .expect("loads");
    let err = rules.call::<_, serde_json::Value>("f", ()).unwrap_err();
    assert_eq!(code(&err), "E0325", "{err}");
}

/// D33 §3. Memory is budgeted separately from steps: a call can allocate a
/// great deal in few steps.
#[test]
fn the_memory_budget_stops_a_call_that_allocates_too_much() {
    let rules = Script::builder()
        .source(
            "t.aura",
            "pub def f()\n  xs: range(1000000).map (x, i) -> \"#{x}#{x}#{x}\" end\nend\n",
        )
        .budget(Budget::per_call().memory_mb(4))
        .load()
        .expect("loads");
    let err = rules.call::<_, serde_json::Value>("f", ()).unwrap_err();
    assert_eq!(code(&err), "E0326", "{err}");
}

/// D33 §3. The budget is counted in steps, not time, so the outcome is a
/// property of the script and its input — not of the machine or its load.
#[test]
fn the_budget_is_deterministic() {
    let src = "pub def f(n)\n  xs: range(n).map (x, i) -> x * 2 end\nend\n";
    let tight = |n: i64| {
        Script::builder()
            .source("t.aura", src)
            .budget(Budget::per_call().steps(10_000))
            .load()
            .expect("loads")
            .call::<_, serde_json::Value>("f", (n,))
            .map_err(|e| e.code())
    };
    // The same input gives the same verdict every time.
    let first = tight(4_000);
    for _ in 0..20 {
        assert_eq!(tight(4_000), first);
    }
    // And a large enough input is refused, a small enough one is not.
    assert!(tight(10).is_ok());
    assert_eq!(tight(1_000_000), Err("E0325"));
}

/// D33 §3. There is a default budget, and it is finite: a host that configures
/// nothing is protected all the same. Either counter may be the one that stops
/// it; the clause is that something does.
#[test]
fn the_default_budget_is_finite() {
    let rules = load(concat!(
        "pub def f()\n",
        "  x: range(1000).map (a, i) -> range(1000).map (b, j) -> ",
        "range(1000).map (c, k) -> 1 end end end\n",
        "end\n",
    ));
    let err = rules.call::<_, serde_json::Value>("f", ()).unwrap_err();
    assert!(matches!(code(&err), "E0325" | "E0326"), "{err}");
}

/// D33 §5. A call that fails does not poison the next one.
#[test]
fn a_failed_call_leaves_the_script_usable() {
    let rules = Script::builder()
        .source(
            "t.aura",
            // The assertion comes first: `range(-1)` has an error of its own,
            // which would otherwise answer before the assertion is reached.
            // `map` rather than a bare `range(n)`: steps count expressions, and
            // `range` is one, however long the list it builds — that is paid in
            // bytes. A lambda per element is what spends steps.
            "pub def f(n)\n  assert n >= 0, \"n must not be negative\"\n  xs: range(n).map (x, i) -> x end.len()\nend\n",
        )
        .budget(Budget::per_call().steps(10_000))
        .load()
        .expect("loads");

    assert_eq!(
        code(&rules.call::<_, serde_json::Value>("f", (-1,)).unwrap_err()),
        "E0530"
    );
    assert_eq!(
        code(
            &rules
                .call::<_, serde_json::Value>("f", (1_000_000,))
                .unwrap_err()
        ),
        "E0325"
    );

    #[derive(Deserialize)]
    struct Out {
        xs: i64,
    }
    let ok: Out = rules.call("f", (5,)).expect("a later call is unaffected");
    assert_eq!(ok.xs, 5);
}

// ===========================================================================
// 2. Resources
// ===========================================================================

// D33 §6 — memory per call is constant — is tested in `script_memory.rs`.
// It installs a counting global allocator, and a global allocator is per test
// binary, so it cannot share this file without counting every other test too.

/// D33 §8. One instance serves every thread.
#[test]
fn a_script_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Script>();
}

/// D33 §8. Concurrent calls on one instance agree with sequential ones.
#[test]
fn concurrent_calls_agree() {
    let rules = std::sync::Arc::new(load("pub def double(x)\n  v: x * 2\nend\n"));
    #[derive(Deserialize)]
    struct V {
        v: i64,
    }
    let handles: Vec<_> = (0..8)
        .map(|t| {
            let rules = rules.clone();
            std::thread::spawn(move || {
                for i in 0..1_000i64 {
                    let out: V = rules.call("double", (i + t,)).expect("call");
                    assert_eq!(out.v, (i + t) * 2);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread");
    }
}

// ===========================================================================
// 3. The boundary with the host
// ===========================================================================

/// D33 §E00xx. `function()` checks the name, the kind and the arity once, at
/// bind time — so a mistake fails when the service starts, not on a request.
#[test]
fn binding_a_function_checks_it_once() {
    let rules = load("pub def f(a, b)\n  v: a\nend\nversion: \"1\"\n");

    let missing = rules.function::<(i64, i64), serde_json::Value>("g");
    assert_eq!(code(&missing.unwrap_err()), "E0002");

    let not_fn = rules.function::<(i64, i64), serde_json::Value>("version");
    assert_eq!(code(&not_fn.unwrap_err()), "E0003");

    let arity = rules.function::<(i64,), serde_json::Value>("f");
    assert_eq!(code(&arity.unwrap_err()), "E0004");

    assert!(rules.function::<(i64, i64), serde_json::Value>("f").is_ok());
}

/// D33 §4. A value Aura cannot hold exactly is refused at the boundary rather
/// than wrapped or rounded into something else.
#[test]
fn the_boundary_refuses_what_it_cannot_carry() {
    let rules = load("pub def id(x)\n  v: x\nend\n");
    let too_big = rules.call::<_, serde_json::Value>("id", (u64::MAX,));
    assert_eq!(code(&too_big.unwrap_err()), "E0005");
    let nan = rules.call::<_, serde_json::Value>("id", (f64::NAN,));
    assert_eq!(code(&nan.unwrap_err()), "E0005");
}

/// D33 §4. A result that does not fit the host's type names where it did not.
#[test]
fn a_result_of_the_wrong_shape_names_the_path() {
    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct Out {
        inner: Inner,
    }
    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct Inner {
        port: u16,
    }
    let rules = load("pub def f()\n  inner:\n    port: \"eighty\"\n  end\nend\n");
    let err = rules.call::<_, Out>("f", ()).unwrap_err();
    assert_eq!(code(&err), "E0006");
    assert!(err.to_string().contains("inner.port"), "{err}");
}

/// Host types cross in both directions without the host naming any compiler
/// internals, and key order survives (determinism reaches the host too).
#[test]
fn host_types_cross_both_ways() {
    #[derive(Serialize)]
    struct Event {
        device: &'static str,
        temp_c: f64,
    }
    #[derive(Deserialize, Debug, PartialEq)]
    struct Summary {
        total: u64,
        alerts: u64,
    }
    let rules = load(concat!(
        "pub def summarize(events)\n",
        "  total:  events.len()\n",
        "  alerts: events.filter (e, i) -> e.temp_c > 80.0 end.len()\n",
        "end\n",
    ));
    let summarize = rules
        .function::<(&[Event],), Summary>("summarize")
        .expect("binds");
    let events = [
        Event {
            device: "a",
            temp_c: 20.0,
        },
        Event {
            device: "b",
            temp_c: 85.5,
        },
    ];
    assert_eq!(
        summarize.call((&events[..],)).expect("call"),
        Summary {
            total: 2,
            alerts: 1
        }
    );

    let keys: serde_json::Value = rules.call("summarize", (&events[..],)).expect("call");
    let order: Vec<_> = keys.as_object().expect("object").keys().cloned().collect();
    assert_eq!(order, ["total", "alerts"]);
}

/// Errors carry what the CLI prints: a code, a position, and the help line.
#[test]
fn errors_read_like_the_cli() {
    let e = load_err("pub def f()\n  v: 1 +\nend\n");
    assert!(e.code().starts_with("E02"), "{e}");
    let text = e.to_string();
    assert!(text.contains("t.aura:"), "no position in: {text}");
}
