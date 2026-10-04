use super::run_both_modes;

#[test]
fn conditional_expressions_preserve_branch_checks_and_control_flow() {
    run_both_modes(
        include_str!("../../../../tests/language/conditional-expressions.whim"),
        "/conditional-expressions.whim",
    );
}
