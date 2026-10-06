use whim_compiler::CompileConfiguration;
use whim_compiler::CompileErrorKind;
use whim_compiler::compile_with_configuration;
use whim_syn::arena::LocalArena;
use whim_syn::parser::parse;
use whim_value::heap::Heap;

#[test]
fn fresh_is_reserved_and_generation_takes_no_arguments() {
    for source in [
        "function fresh(): void {}",
        "class fresh {}",
        "interface fresh {}",
        "type fresh = int;",
        "newtype fresh = uint;",
        "const fresh = 1;",
        "fresh!(1);",
        "fresh!(,);",
        "fresh(1);",
    ] {
        let arena = LocalArena::new();
        assert!(parse(&arena, source).is_err(), "{source}");
    }
}

#[test]
fn fresh_cannot_be_created_in_constant_expressions() {
    for (source, kind) in [
        (
            "const TOKEN = fresh!();",
            CompileErrorKind::NonConstantInitializer,
        ),
        (
            "function f(fresh $value = fresh!()): void {}",
            CompileErrorKind::NonConstantParameterDefault,
        ),
        (
            "class C { public fresh $token = fresh!(); }",
            CompileErrorKind::NonConstantPropertyDefault,
        ),
        (
            "#[A(fresh!())] function f(): void {}",
            CompileErrorKind::NonConstantAttributeArgument,
        ),
    ] {
        let arena = LocalArena::new();
        let heap = Heap::new();
        let result = compile_with_configuration(
            parse(&arena, source).unwrap(),
            "/fresh.whim",
            &heap,
            CompileConfiguration::default(),
        );
        assert_eq!(result.unwrap_err().kind, kind, "{source}");
    }
}
