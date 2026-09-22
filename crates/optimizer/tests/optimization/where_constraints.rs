use super::compile;
use super::method;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::ArrayValueMode;
use whim_optimizer::OptimizationConfiguration;

#[test]
fn callable_bounds_specialize_bodies_and_proven_calls() {
    let unit = compile(
        r"
        function inline_bound<T: int>(T $a, T $b): int { return $a + $b; }
        function where_bound<T>(T $a, T $b): int where T: int { return $a + $b; }
        $closure = fn<T>(T $a, T $b): int where T: int => $a + $b;
        assert!(where_bound::<int>(2, 3) == 5);
        where_bound::<float>(2.0, 3.0);
        ",
        OptimizationConfiguration::default(),
    );
    for function in &unit.functions {
        let name = function.name.as_bytes();
        if name == b"inline_bound" || name == b"where_bound" || name.starts_with(b"{closure:") {
            assert!(
                function
                    .chunk
                    .code
                    .iter()
                    .any(|instruction| matches!(instruction, Instruction::IntAdd { .. }))
            );
            assert!(!function.chunk.code.iter().any(|instruction| matches!(
                instruction,
                Instruction::Add { .. } | Instruction::Return { .. }
            )));
        }
    }
    let inline = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"inline_bound")
        .unwrap();
    let clause = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"where_bound")
        .unwrap();
    assert_eq!(inline.chunk.code, clause.chunk.code);
    assert_eq!(
        unit.main
            .code
            .iter()
            .filter(|instruction| matches!(
                instruction,
                Instruction::CallNamed { .. } | Instruction::CallNamedUnchecked { .. }
            ))
            .count(),
        1
    );
    assert!(
        !unit
            .main
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CheckWhereConstraints))
    );
}

#[test]
fn where_bounds_specialize_parameters_properties_and_collection_elements() {
    let unit = compile(
        r"
        type Numbers = int|float;
        class Vector<T> {
            public function __construct(private vec<T> $values) {}
            public function sum(): int where T: Numbers, T: int {
                $sum = 0;
                foreach ($this->values as $value) { $sum += $value; }
                return $sum;
            }
            public function add(T $left, T $right): int where T: int {
                return $left + $right;
            }
            public function floating<U>(U $left, U $right): float where U: float {
                return $left + $right;
            }
            public function first<U>(U $values): int where U: vec<T>, T: int {
                foreach ($values as $value) { return $value; }
                return 0;
            }
            public function index<U>(U $values, int $index): int where U: vec<T>, T: int {
                return $values[$index] + 1;
            }
            public function optional(T $value = 1): int where T: int { return $value + 1; }
            public function widen(): int|float where T: int {
                $values = $this->values;
                $values[] = 0.5;
                $sum = 0;
                foreach ($values as $value) { $sum += $value; }
                return $sum;
            }
            public function plain(T $left, T $right): mixed { return $left + $right; }
        }
        ",
        OptimizationConfiguration::default(),
    );

    for name in [
        b"Vector::sum".as_slice(),
        b"Vector::add",
        b"Vector::floating",
        b"Vector::first",
        b"Vector::index",
        b"Vector::optional",
    ] {
        let code = method(&unit, name);
        assert_eq!(
            matches!(code[0], Instruction::CheckWhereConstraints),
            name != b"Vector::floating"
        );
        assert!(!code.iter().any(|instruction| matches!(
            instruction,
            Instruction::Add { .. } | Instruction::Return { .. }
        )));

        assert!(
            code.iter().any(|instruction| matches!(
                instruction,
                Instruction::ReturnScalarUnchecked { .. }
            ))
        );
    }

    assert!(
        method(&unit, b"Vector::sum")
            .iter()
            .any(|instruction| matches!(
                instruction,
                Instruction::VecForeachNext {
                    value_mode: ArrayValueMode::Int,
                    ..
                }
            ))
    );
    assert!(
        method(&unit, b"Vector::floating")
            .iter()
            .any(|instruction| matches!(instruction, Instruction::FloatAdd { .. }))
    );
    assert!(
        method(&unit, b"Vector::plain")
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Add { .. }))
    );
    let widened = method(&unit, b"Vector::widen");
    assert!(
        widened
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Add { .. }))
    );
    assert!(!widened.iter().any(|instruction| matches!(
        instruction,
        Instruction::VecForeachNext {
            value_mode: ArrayValueMode::Int | ArrayValueMode::Float,
            ..
        }
    )));
}

#[test]
fn proven_where_constraints_allow_instance_and_static_inlining() {
    let unit = compile(
        r"
        final class Number<T> {
            public function __construct(private T $value) {}
            public function value(): int where T: int { return $this->value; }
        }
        final class Math {
            public static function add<T>(T $a, T $b): int where T: int { return $a + $b; }
        }
        function value(Number<int> $number): int { return $number->value(); }
        function add(int $value): int { return Math::add::<int>($value, 1); }
        ",
        OptimizationConfiguration::default(),
    );

    for name in [b"value".as_slice(), b"add"] {
        let code = &unit
            .functions
            .iter()
            .find(|function| function.name.as_bytes() == name)
            .unwrap()
            .chunk
            .code;

        assert!(
            !code.iter().any(|instruction| matches!(
                instruction,
                Instruction::CallMethod { .. }
                    | Instruction::CallMethodDirect { .. }
                    | Instruction::CallMethodUnchecked { .. }
                    | Instruction::CallStatic { .. }
                    | Instruction::CheckWhereConstraints
            )),
            "{name:?}: {code:?}"
        );
    }

    assert_eq!(
        method(&unit, b"Number::value")[0],
        Instruction::CheckWhereConstraints
    );
}

#[test]
fn where_upper_bounds_do_not_widen_return_types_or_shadowed_properties() {
    let unit = compile(
        r"
        final class Box<T> {
            public function __construct(private T $value) {}
            public function wrong(T $value): T where T: int { return $value + 1; }
            public function shadow<T>(T $argument): bool where T: string {
                return $this->value is string;
            }
            public function invariant(Box<T> $box): Box<int> where T: int { return $box; }
            public function callback(fn(T): int $callback): fn(int): int where T: int { return $callback; }
            public function negative(!T $value): !int where T: int { return $value; }
        }
        ",
        OptimizationConfiguration::default(),
    );

    for name in [
        b"Box::wrong".as_slice(),
        b"Box::invariant",
        b"Box::callback",
        b"Box::negative",
    ] {
        assert!(
            method(&unit, name)
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Return { .. })),
            "{name:?}"
        );
    }

    assert!(
        method(&unit, b"Box::shadow")
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Is { .. }))
    );
}

#[test]
fn inlining_keeps_where_checks_in_the_callee_environment() {
    let unit = compile(
        r"
        final class Box<T> {
            #[Whim\Marker\AlwaysInline]
            public function get(): int where T: int { return 1; }

            public function call(): int { return $this->get(); }
        }
        final class Choose {
            #[Whim\Marker\AlwaysInline]
            public static function choose<U>(): int where U: int { return 2; }
        }
        function choose(): int { return Choose::choose::<string>(); }
        $box = new Box::<string>();
        $box->get();
        ",
        OptimizationConfiguration::default(),
    );

    assert_eq!(
        method(&unit, b"Box::get")[0],
        Instruction::CheckWhereConstraints
    );
    assert!(
        method(&unit, b"Box::call")
            .iter()
            .any(|instruction| matches!(
                instruction,
                Instruction::CallMethod { .. }
                    | Instruction::CallMethodDirect { .. }
                    | Instruction::CallMethodUnchecked { .. }
            ))
    );
    let choose = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"choose")
        .unwrap();
    assert!(
        choose
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CallStatic { .. }))
    );
    assert!(unit.main.code.iter().any(|instruction| matches!(
        instruction,
        Instruction::CallMethod { .. }
            | Instruction::CallMethodDirect { .. }
            | Instruction::CallMethodUnchecked { .. }
            | Instruction::CallMethodDiscarded { .. }
    )));
}
