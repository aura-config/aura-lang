//! Host functions, as D35 promises them.
//!
//! Written before the implementation, against the designed API, one test per
//! decision. Security first, as the decision ranks it. A test weakened to make
//! an implementation pass is a clause of D35 quietly withdrawn.

use aura_lang::{Budget, Error, Script};
use serde::Deserialize;

#[derive(Deserialize, Debug, PartialEq)]
struct V<T> {
    v: T,
}

fn code(e: &Error) -> &'static str {
    e.code()
}

// ===========================================================================
// The ordinary case
// ===========================================================================

/// D35 §1. The script imports a host module and calls into it like any module.
#[test]
fn a_script_calls_a_host_function() {
    let rules = Script::builder()
        .source(
            "t.aura",
            "import \"host:geo\" as geo\npub def f(ip)\n  v: geo.region(ip)\nend\n",
        )
        .pure("geo.region", |ip: String| -> Option<String> {
            (ip == "203.0.113.7").then(|| "eu".to_string())
        })
        .load()
        .expect("loads");
    let r: V<String> = rules.call("f", ("203.0.113.7",)).expect("call");
    assert_eq!(r.v, "eu");
}

/// D35 §4. `None` is `null` to the script, which decides what absence means.
#[test]
fn none_reaches_the_script_as_null() {
    let rules = Script::builder()
        .source(
            "t.aura",
            concat!(
                "import \"host:geo\" as geo\n",
                "pub def f(ip)\n",
                "  r = geo.region(ip)\n",
                "  v: r == null ? \"global\" : r\n",
                "end\n",
            ),
        )
        .pure("geo.region", |_ip: String| -> Option<String> { None })
        .load()
        .expect("loads");
    let r: V<String> = rules.call("f", ("192.0.2.1",)).expect("call");
    assert_eq!(r.v, "global");
}

/// Host functions take any number of arguments, as `call` passes any tuple.
#[test]
fn a_host_function_takes_several_arguments() {
    let rules = Script::builder()
        .source(
            "t.aura",
            "import \"host:math\" as m\npub def f()\n  v: m.add(2, 3)\nend\n",
        )
        .pure("math.add", |a: i64, b: i64| a + b)
        .load()
        .expect("loads");
    let r: V<i64> = rules.call("f", ()).expect("call");
    assert_eq!(r.v, 5);
}

// ===========================================================================
// 1. Security
// ===========================================================================

/// D35 §2. Registering only pure functions grants nothing: the load is still
/// hermetic, so `env()` is still refused at load.
#[test]
fn pure_functions_leave_the_script_hermetic() {
    let err = Script::builder()
        .source(
            "t.aura",
            concat!(
                "import \"host:geo\" as geo\n",
                "pub def f(ip)\n",
                "  v: geo.region(ip)\n",
                "  h: env(\"HOME\", \"-\")\n",
                "end\n",
            ),
        )
        .pure("geo.region", |_ip: String| -> Option<String> { None })
        .load()
        .unwrap_err();
    assert_eq!(code(&err), "E0505", "{err}");
}

/// D35 §2. An effect function is a grant - of itself, not of anything else.
#[test]
fn an_effect_grants_only_itself() {
    let rules = Script::builder()
        .source(
            "t.aura",
            concat!(
                "import \"host:db\" as db\n",
                "pub def f()\n",
                "  u: db.user(1)\n",
                "  h: env(\"HOME\", \"-\")\n",
                "end\n",
            ),
        )
        .effect("db.user", |id: i64| -> Result<String, String> {
            Ok(format!("user-{id}"))
        })
        .load()
        .expect("loads: the effect is granted, so the load is not hermetic");
    let err = rules.call::<_, serde_json::Value>("f", ()).unwrap_err();
    assert_eq!(code(&err), "E0310", "{err}");
}

/// D35 §3. An imported module cannot call a host function, even one granted
/// to the root - a third-party package must not reach the host's services.
#[test]
fn host_functions_do_not_reach_imports() {
    let rules = Script::builder()
        .source(
            "lib.aura",
            "import \"host:db\" as db\npub def leak()\n  v: db.user(1)\nend\n",
        )
        .source(
            "main.aura",
            "import \"./lib.aura\" as lib\npub def f()\n  v: lib.leak()\nend\n",
        )
        .entry("main.aura")
        .effect("db.user", |id: i64| -> Result<i64, String> { Ok(id) })
        .load();
    let err = rules.unwrap_err();
    assert_eq!(code(&err), "E0310", "{err}");
}

/// D35 §3. ... unless the host grants that one function to imports explicitly.
#[test]
fn a_host_function_can_be_granted_to_imports_explicitly() {
    let rules = Script::builder()
        .source(
            "lib.aura",
            "import \"host:geo\" as geo\npub def r(ip)\n  v: geo.region(ip)\nend\n",
        )
        .source(
            "main.aura",
            "import \"./lib.aura\" as lib\npub def f()\n  v: lib.r(\"x\")\nend\n",
        )
        .entry("main.aura")
        .pure("geo.region", |_ip: String| "eu".to_string())
        .grant_to_imports("geo.region")
        .load()
        .expect("loads");
    let r: serde_json::Value = rules.call("f", ()).expect("call");
    assert_eq!(r["v"]["v"], "eu");
}

/// D35 §6. A host function may not call back into the script. Re-entrancy
/// across the boundary is the class of defect behind the 2016 DAO exploit, and
/// the API alone cannot prevent it - a closure can capture the script - so it
/// is refused at run time.
#[test]
fn a_host_function_cannot_call_back_into_the_script() {
    use std::sync::{Arc, OnceLock};
    let slot: Arc<OnceLock<Script>> = Arc::new(OnceLock::new());
    let inner = slot.clone();
    let rules = Script::builder()
        .source(
            "t.aura",
            concat!(
                "import \"host:back\" as back\n",
                "pub def f()\n  v: back.call()\nend\n",
                "pub def g()\n  v: 1\nend\n",
            ),
        )
        .pure("back.call", move || -> i64 {
            let s = inner.get().expect("set");
            match s.call::<_, serde_json::Value>("g", ()) {
                Ok(_) => 1,
                Err(e) => {
                    assert_eq!(e.code(), "E0010");
                    0
                }
            }
        })
        .load()
        .expect("loads");
    slot.set(rules).ok();
    let s = slot.get().expect("set");
    let r: V<i64> = s.call("f", ()).expect("the outer call completes");
    assert_eq!(r.v, 0, "the inner call must have been refused");
}

// ===========================================================================
// 2. Checked at load
// ===========================================================================

/// D35 §8. A module the host does not provide fails the load.
#[test]
fn an_unprovided_host_module_fails_the_load() {
    let err = Script::builder()
        .source(
            "t.aura",
            "import \"host:geo\" as geo\npub def f(ip)\n  v: geo.region(ip)\nend\n",
        )
        .load()
        .unwrap_err();
    assert_eq!(code(&err), "E0007", "{err}");
}

/// D35 §8. A function the host does not provide fails the load too - on a
/// branch that would never run, which is what makes it a check and not a
/// surprise.
#[test]
fn an_unprovided_host_function_fails_the_load() {
    let err = Script::builder()
        .source(
            "t.aura",
            concat!(
                "import \"host:geo\" as geo\n",
                "pub def f(ip)\n",
                "  v: ip == \"never\" ? geo.city(ip) : geo.region(ip)\n",
                "end\n",
            ),
        )
        .pure("geo.region", |_ip: String| "eu".to_string())
        .load()
        .unwrap_err();
    assert_eq!(code(&err), "E0007", "{err}");
    assert!(err.to_string().contains("geo.city"), "{err}");
}

/// D35 §9. A direct call with the wrong number of arguments fails the load.
#[test]
fn a_wrong_arity_fails_the_load() {
    let err = Script::builder()
        .source(
            "t.aura",
            "import \"host:geo\" as geo\npub def f(ip)\n  v: geo.region(ip, 2)\nend\n",
        )
        .pure("geo.region", |_ip: String| "eu".to_string())
        .load()
        .unwrap_err();
    assert_eq!(code(&err), "E0009", "{err}");
}

/// D35 §9. Types are checked when the call is made, and the error names the
/// function.
#[test]
fn a_wrong_argument_type_is_e0009_at_the_call() {
    let rules = Script::builder()
        .source(
            "t.aura",
            "import \"host:math\" as m\npub def f()\n  v: m.double(\"two\")\nend\n",
        )
        .pure("math.double", |n: i64| n * 2)
        .load()
        .expect("loads: types are a call-time check");
    let err = rules.call::<_, serde_json::Value>("f", ()).unwrap_err();
    assert_eq!(code(&err), "E0009", "{err}");
    assert!(err.to_string().contains("math.double"), "{err}");
}

// ===========================================================================
// 3. Errors and budget
// ===========================================================================

/// D35 §4. `Err` from the host stops the call with E0008 and the host's message.
#[test]
fn a_host_error_stops_the_call() {
    let rules = Script::builder()
        .source(
            "t.aura",
            "import \"host:db\" as db\npub def f()\n  v: db.user(1)\nend\n",
        )
        .effect("db.user", |_id: i64| -> Result<String, String> {
            Err("database unavailable".to_string())
        })
        .load()
        .expect("loads");
    let err = rules.call::<_, serde_json::Value>("f", ()).unwrap_err();
    assert_eq!(code(&err), "E0008", "{err}");
    assert!(err.to_string().contains("database unavailable"), "{err}");
}

/// D35 §5. A host call costs steps, so a loop of them is stopped by the budget
/// even though the time spent inside the host is invisible to it.
#[test]
fn host_calls_are_charged_to_the_step_budget() {
    let rules = Script::builder()
        .source(
            "t.aura",
            concat!(
                "import \"host:math\" as m\n",
                "pub def f()\n",
                "  v: range(100000).map (x, i) -> m.double(x) end.len()\n",
                "end\n",
            ),
        )
        .pure("math.double", |n: i64| n * 2)
        .host_call_steps(100)
        .budget(Budget::per_call().steps(1_000_000).memory_bytes(u64::MAX))
        .load()
        .expect("loads");
    let err = rules.call::<_, serde_json::Value>("f", ()).unwrap_err();
    assert_eq!(code(&err), "E0325", "{err}");
}

/// The script stays `Send + Sync` with host functions registered.
#[test]
fn a_script_with_host_functions_is_send_and_sync() {
    fn check<T: Send + Sync>(_: &T) {}
    let rules = Script::builder()
        .source(
            "t.aura",
            "import \"host:geo\" as geo\npub def f(ip)\n  v: geo.region(ip)\nend\n",
        )
        .pure("geo.region", |_ip: String| "eu".to_string())
        .load()
        .expect("loads");
    check(&rules);
}

// ===========================================================================
// 4. What the load must not miss
// ===========================================================================

/// D35 §8. Interpolations are parsed lazily, so the load has to look inside
/// them: a call hidden in `"#{...}"` is checked like any other.
#[test]
fn a_call_inside_an_interpolation_is_checked_at_load() {
    let err = Script::builder()
        .source(
            "t.aura",
            "import \"host:geo\" as geo\npub def f(ip)\n  v: \"at #{geo.city(ip)}\"\nend\n",
        )
        .pure("geo.region", |_ip: String| "eu".to_string())
        .load()
        .unwrap_err();
    assert_eq!(code(&err), "E0007", "{err}");
    assert!(err.to_string().contains("geo.city"), "{err}");
}

/// D35 §3. An import that passes the module around instead of calling it
/// directly is still refused: the load cannot see which function it will call,
/// so it takes the use to reach all of them.
#[test]
fn a_host_module_passed_as_a_value_does_not_reach_imports() {
    let err = Script::builder()
        .source(
            "lib.aura",
            concat!(
                "import \"host:db\" as db\n",
                "pub def leak()\n",
                "  m = db\n",
                "  v: m.user(1)\n",
                "end\n",
            ),
        )
        .source(
            "main.aura",
            "import \"./lib.aura\" as lib\npub def f()\n  v: lib.leak()\nend\n",
        )
        .entry("main.aura")
        .effect("db.user", |id: i64| -> Result<i64, String> { Ok(id) })
        .load()
        .unwrap_err();
    assert_eq!(code(&err), "E0310", "{err}");
}

/// Built-in object methods win over an object's fields, so a host function
/// named `keys` could never be called. Refused when registered, not shadowed.
#[test]
fn a_host_function_named_like_a_builtin_method_is_refused() {
    let err = Script::builder()
        .source("t.aura", "x: 1\n")
        .pure("geo.keys", || 1)
        .load()
        .unwrap_err();
    assert_eq!(code(&err), "E0001", "{err}");
    assert!(err.to_string().contains("geo.keys"), "{err}");
}

/// D35 §9. A result crosses the boundary by the same exact rules as an
/// argument: a NaN is refused, not turned into `null`.
#[test]
fn a_result_aura_cannot_hold_is_e0009() {
    let rules = Script::builder()
        .source(
            "t.aura",
            "import \"host:m\" as m\npub def f()\n  v: m.nan()\nend\n",
        )
        .pure("m.nan", || f64::NAN)
        .load()
        .expect("loads");
    let err = rules.call::<_, serde_json::Value>("f", ()).unwrap_err();
    assert_eq!(code(&err), "E0009", "{err}");
    assert!(err.to_string().contains("m.nan"), "{err}");
}
