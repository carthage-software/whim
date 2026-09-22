use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_compiler::CompileConfiguration;
use whim_compiler::CompileErrorKind;
use whim_compiler::compile_with_configuration;
use whim_optimizer::OptimizationConfiguration;
use whim_syn::arena::LocalArena;
use whim_syn::parser::parse;
use whim_value::heap::Heap;

#[test]
fn method_where_metadata_preserves_parameters_bounds_order_and_spans() {
    let source =
        "class Box<T> { public function check<U>(): void where T: int, U: vec<T>, T: string {} }";
    let arena = LocalArena::new();
    let program = parse(&arena, source).unwrap();
    for enabled in [false, true] {
        let heap = Heap::new();
        let unit = compile_with_configuration(
            program,
            "/where.whim",
            &heap,
            CompileConfiguration {
                optimization: OptimizationConfiguration {
                    enabled,
                    ..OptimizationConfiguration::default()
                },
                ..CompileConfiguration::default()
            },
        )
        .unwrap();
        let constraints = &unit.classes[0].methods[0].where_constraints;
        assert_eq!(
            unit.classes[0].methods[0].function.chunk.code[0],
            Instruction::CheckWhereConstraints
        );
        let expected = [("T", "T: int"), ("U", "U: vec<T>"), ("T", "T: string")];
        assert_eq!(constraints.len(), expected.len());
        for (constraint, (parameter, text)) in constraints.iter().zip(expected) {
            assert_eq!(constraint.parameter.as_bytes(), parameter.as_bytes());
            assert_eq!(
                &source[constraint.span.start.offset as usize..constraint.span.end.offset as usize],
                text,
            );
        }
        assert!(matches!(constraints[0].bound, TypeDescriptor::Int));
        assert!(
            matches!(&constraints[1].bound, TypeDescriptor::Vector(Some(element))
            if matches!(element.as_ref(), TypeDescriptor::Parameter(name) if name.as_bytes() == b"T"))
        );
        assert!(matches!(constraints[2].bound, TypeDescriptor::String));
    }
}

#[test]
fn where_metadata_requires_an_available_parameter_and_a_valid_bound() {
    for (source, kind) in [
        (
            "class Box<T> { public function check(): void where Missing: int {} }",
            CompileErrorKind::UnknownWhereConstraintParameter,
        ),
        (
            "class Box<T> { public static function check(): void where T: int {} }",
            CompileErrorKind::ClassTypeParameterInStaticMember,
        ),
        (
            "class Box<T> { public static function check<U>(): void where U: T {} }",
            CompileErrorKind::ClassTypeParameterInStaticMember,
        ),
        (
            "class Box<T> { public function check(): void where T: void {} }",
            CompileErrorKind::ReturnOnlyType,
        ),
        (
            "class Box<T> { public function check(): void where T: vec<_> {} }",
            CompileErrorKind::WildcardTypeArgument,
        ),
    ] {
        let arena = LocalArena::new();
        let heap = Heap::new();
        let error = compile_with_configuration(
            parse(&arena, source).unwrap(),
            "/invalid-where.whim",
            &heap,
            CompileConfiguration::default(),
        )
        .unwrap_err();
        assert_eq!(error.kind, kind, "{source}: {error:?}");
    }
}

#[test]
fn where_check_precedes_defaults_and_promoted_properties() {
    let source = "function initial(): int { return 1; }
        class Box<T> { public function __construct(public int $value = initial()) where T: int {} }";
    for enabled in [false, true] {
        let arena = LocalArena::new();
        let heap = Heap::new();
        let unit = compile_with_configuration(
            parse(&arena, source).unwrap(),
            "/where-constructor.whim",
            &heap,
            CompileConfiguration {
                optimization: OptimizationConfiguration {
                    enabled,
                    ..OptimizationConfiguration::default()
                },
                ..CompileConfiguration::default()
            },
        )
        .unwrap();
        assert_eq!(
            unit.classes[0].methods[0].function.chunk.code[0],
            Instruction::CheckWhereConstraints
        );
    }
}
