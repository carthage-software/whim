use whim_compiler::CompileConfiguration;
use whim_compiler::CompileErrorKind;
use whim_compiler::compile_with_configuration;
use whim_syn::arena::LocalArena;
use whim_syn::parser::parse;
use whim_value::heap::Heap;

#[test]
fn unsigned_literals_and_ranges_reject_invalid_signedness() {
    for (source, kind) in [
        ("$value = -24u;", CompileErrorKind::NegativeUnsignedLiteral),
        (
            "if (false) { $value = -24U; }",
            CompileErrorKind::NegativeUnsignedLiteral,
        ),
        (
            "const VALUE = -1u;",
            CompileErrorKind::NegativeUnsignedLiteral,
        ),
        (
            "function f(uint $value = -1u): void {}",
            CompileErrorKind::NegativeUnsignedLiteral,
        ),
        ("type T = -1u;", CompileErrorKind::NegativeUnsignedLiteral),
        ("type T = -1u..;", CompileErrorKind::NegativeUnsignedLiteral),
        (
            "$value = match (dict[]) { dict[-1u => $_, ...] => 1, _ => 2 };",
            CompileErrorKind::NegativeUnsignedLiteral,
        ),
        (
            "$value = match (1u) { -1u => 0, _ => 1 };",
            CompileErrorKind::NegativeUnsignedLiteral,
        ),
        (
            "type T = 1..=10u;",
            CompileErrorKind::MixedIntegerRangeBounds,
        ),
        (
            "type T = 1u..10;",
            CompileErrorKind::MixedIntegerRangeBounds,
        ),
        (
            "enum E: uint { case A = 1; }",
            CompileErrorKind::EnumCaseValueMismatch,
        ),
        (
            "enum E: int { case A = 1u; }",
            CompileErrorKind::EnumCaseValueMismatch,
        ),
        (
            "enum E: uint { case A = 1u; case B = 1U; }",
            CompileErrorKind::DuplicateEnumCaseValue,
        ),
    ] {
        let arena = LocalArena::new();
        let heap = Heap::new();
        let result = compile_with_configuration(
            parse(&arena, source).unwrap(),
            "/uint.whim",
            &heap,
            CompileConfiguration::default(),
        );
        assert_eq!(result.unwrap_err().kind, kind, "{source}");
    }
}

#[test]
fn unsigned_string_lengths_are_rejected() {
    for source in [
        "type T = string[1u];",
        "type T = string[1u..];",
        "type T = string[..=10u];",
    ] {
        let arena = LocalArena::new();
        let heap = Heap::new();
        let result = compile_with_configuration(
            parse(&arena, source).unwrap(),
            "/uint-length.whim",
            &heap,
            CompileConfiguration::default(),
        );
        assert!(result.is_err(), "{source}");
    }
}
