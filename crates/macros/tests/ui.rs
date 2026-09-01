#[test]
fn macro_diagnostics_stay_pinned() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/*.rs");
}
