//! The checkout example is documentation that runs, so it is run here.
//!
//! It shows D33, D35 and D36 working together on a real shape of problem, and
//! asserts every outcome it prints: what each order gets, that a valid edit
//! is picked up, and that a contract break and an ungranted capability are
//! refused while the old rules keep serving. If any of that stops being true,
//! the guide is lying, and this test says so before a reader does.

#[path = "../examples/checkout/main.rs"]
mod checkout;

#[test]
fn the_checkout_example_runs_as_documented() {
    checkout::main();
}

/// The rule files are Aura a reader will copy, so they are in canonical form,
/// like every manifest under `examples/`.
#[test]
fn the_checkout_rules_are_formatted() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/checkout");
    for f in ["checkout.aura", "money.aura"] {
        let src = std::fs::read_to_string(dir.join(f)).expect("read");
        let formatted = aura_lang::fmt::format_source(&src).expect("parses");
        assert_eq!(formatted, src, "{f} is not formatted");
    }
}
