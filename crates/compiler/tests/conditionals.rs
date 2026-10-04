use whim_compiler::CompileConfiguration;
use whim_compiler::CompileErrorKind;
use whim_compiler::compile_with_configuration;
use whim_syn::arena::LocalArena;
use whim_syn::parser::parse;
use whim_value::heap::Heap;

#[test]
fn unchosen_conditional_branches_still_require_valid_control_flow() {
    for source in ["true ? 1 : break;", "false ? continue : 2;"] {
        let arena = LocalArena::new();
        let heap = Heap::new();
        let result = compile_with_configuration(
            parse(&arena, source).unwrap(),
            "/conditionals.whim",
            &heap,
            CompileConfiguration::default(),
        );
        assert_eq!(
            result.unwrap_err().kind,
            CompileErrorKind::LoopJumpOutsideLoop,
            "{source}",
        );
    }
}

#[test]
fn all_conditional_operands_must_be_constant_in_initializers() {
    for (source, kind) in [
        (
            "const VALUE = $condition ? 1 : 2;",
            CompileErrorKind::NonConstantInitializer,
        ),
        (
            "const VALUE = true ? 1 : $value;",
            CompileErrorKind::NonConstantInitializer,
        ),
        (
            "const VALUE = false ? $value : 2;",
            CompileErrorKind::NonConstantInitializer,
        ),
        (
            "function f($value = true ? 1 : $missing) {}",
            CompileErrorKind::NonConstantParameterDefault,
        ),
    ] {
        let arena = LocalArena::new();
        let heap = Heap::new();
        let result = compile_with_configuration(
            parse(&arena, source).unwrap(),
            "/conditionals.whim",
            &heap,
            CompileConfiguration::default(),
        );
        assert_eq!(result.unwrap_err().kind, kind, "{source}");
    }
}
