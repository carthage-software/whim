use std::ops::Deref;
use std::rc::Rc;

use whim_syn::arena::LocalArena;
use whim_syn::parser::parse;

use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::SwitchTable;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::IndexAddMode;
use crate::bytecode::instruction::operands::PropertyIndexUpdateMode;
use crate::bytecode::instruction::operands::PropertyRemoveMode;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::verify::verify_unit;
use crate::compiler::CompileConfiguration;
use crate::compiler::compile_with_configuration;
use crate::optimizer::OptimizationConfiguration;
use crate::optimizer::World;
use crate::optimizer::callable::optimize_function;
use crate::optimizer::callable::optimize_method;
use crate::value::heap::Heap;

struct OwnedUnit {
    unit: CompiledUnit,
    _heap: Rc<Heap>,
}

impl Deref for OwnedUnit {
    type Target = CompiledUnit;

    fn deref(&self) -> &CompiledUnit {
        &self.unit
    }
}

const CALL_SOURCE: &str = r"
final class Target {
    public function take(int $value): int {
        return $value;
    }
}

final class Caller {
    public function __construct(private Target $target) {}

    public function proven(int $value): int {
        return $this->target->take($value);
    }

    public function unproven(mixed $value): int {
        return $this->target->take($value);
    }
}
";

fn compile(source: &str, optimization: OptimizationConfiguration) -> OwnedUnit {
    let arena = LocalArena::new();
    let program = parse(&arena, source).expect("the test source parses");
    let heap = Heap::new();
    let unit = compile_with_configuration(
        program,
        "/project/optimizer.whim",
        &heap,
        CompileConfiguration {
            optimization,
            trusted_return_types: false,
        },
    )
    .expect("the test source compiles");
    OwnedUnit { unit, _heap: heap }
}

fn method<'a>(unit: &'a CompiledUnit, name: &[u8]) -> &'a [Instruction] {
    &unit
        .classes
        .iter()
        .flat_map(|class| &class.methods)
        .find(|method| method.function.name.as_bytes() == name)
        .expect("the method exists")
        .function
        .chunk
        .code
}

#[test]
fn removes_dead_initial_writes_to_fresh_locals() {
    let unit = compile(
        r"
        $language = 'Whim';
        write_line!('Hello from ' . $language . '!');

        function greet(): void {
            $language = 'Whim';
            write_line!('Hello from ' . $language . '!');
        }
        ",
        OptimizationConfiguration::default(),
    );

    for chunk in [
        &unit.main,
        &unit
            .functions
            .iter()
            .find(|function| function.name.as_bytes() == b"greet")
            .expect("the function exists")
            .chunk,
    ] {
        let loads = chunk
            .code
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::LoadConstant {
                    destination,
                    constant,
                } => Some((*destination, *constant)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(loads.len(), 1);
        let (destination, constant) = loads[0];
        assert_eq!(chunk.register_count, 1);
        assert_eq!(destination.index(), 0);
        assert!(matches!(
            &chunk.constants[usize::from(constant.index())],
            Literal::String(value) if value.as_bytes() == b"Hello from Whim!"
        ));
        assert!(
            chunk.code.iter().any(|instruction| matches!(
                instruction,
                Instruction::WriteLine {
                    value_count,
                    first_value,
                } if value_count.value() == 1 && *first_value == destination
            )),
            "{:?}",
            chunk.code
        );
    }
}

#[test]
fn keeps_dead_writes_that_release_parameters_and_captures() {
    let unit = compile(
        r"
        function overwrite(mixed $value): void {
            $value = null;
            write_line!('done');
        }

        function capture(mixed $value): fn(): void {
            return function() use ($value): void {
                $value = null;
                write_line!('done');
            };
        }
        ",
        OptimizationConfiguration::default(),
    );

    let overwrite = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"overwrite")
        .expect("the function exists");
    assert!(matches!(
        overwrite.chunk.code.first(),
        Some(Instruction::LoadNull { destination }) if destination.index() == 0
    ));

    let closure = unit
        .functions
        .iter()
        .find(|function| !function.capture_types.is_empty())
        .expect("the closure exists");
    assert!(matches!(
        closure.chunk.code.first(),
        Some(Instruction::LoadNull { destination }) if destination.index() == 0
    ));
}

#[test]
fn compiler_emits_canonical_bytecode_without_optimization() {
    let unit = compile(
        r"
        function touch(): null {
            return null;
        }

        final class Box {
            public int $count = 0;
            public dict<int, int> $values = dict[];
        }

        function exercise(
            Box $box,
            vec<int> $items,
            int $limit,
            null|int $optional,
        ): int {
            touch();
            foreach ($items as $item) {
                $box->count++;
                $box->values[$item] += 1;
            }

            for ($index = 0; $index < $limit; $index++) {}
            if ($optional == null) {
                return $limit + 3;
            }

            return $optional;
        }

        function maybe_uninitialized(bool $condition): int {
            if ($condition) {
                $value = 1;
            }

            return $value;
        }
        ",
        OptimizationConfiguration {
            enabled: false,
            ..OptimizationConfiguration::default()
        },
    );
    let exercise = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"exercise")
        .expect("the function exists");
    let code = &exercise.chunk.code;

    assert!(
        code.iter()
            .any(|instruction| matches!(instruction, Instruction::CallNamedDiscarded { .. }))
    );
    assert!(code.iter().any(|instruction| matches!(
        instruction,
        Instruction::ForeachNext {
            key_destination,
            ..
        } if *key_destination == Register::NONE
    )));
    assert!(
        code.iter()
            .any(|instruction| matches!(instruction, Instruction::PropertyStep { .. }))
    );
    assert!(
        code.iter()
            .any(|instruction| matches!(instruction, Instruction::IndexAddAssign { .. }))
    );
    assert!(
        code.iter()
            .any(|instruction| matches!(instruction, Instruction::AddImmediate { .. }))
    );
    assert!(
        code.iter()
            .any(|instruction| matches!(instruction, Instruction::JumpIfNotNull { .. }))
    );
    assert!(
        code.iter()
            .any(|instruction| matches!(instruction, Instruction::JumpUnless { .. }))
    );
    assert!(
        code.iter()
            .any(|instruction| matches!(instruction, Instruction::IncrementJump { .. }))
    );
    let maybe_uninitialized = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"maybe_uninitialized")
        .expect("the function exists");
    let mut checked = maybe_uninitialized
        .chunk
        .code
        .iter()
        .filter_map(|instruction| match instruction {
            Instruction::CheckDefined { subject, .. } => Some(*subject),
            _ => None,
        })
        .collect::<Vec<_>>();
    checked.sort_unstable_by_key(|register| register.index());
    checked.dedup();
    assert_eq!(maybe_uninitialized.chunk.uninitialized_registers, checked);
    verify_unit(&unit).expect("compiler bytecode verifies");
}

#[test]
fn coalesces_reused_loop_initializer_temporaries() {
    let unit = compile(
        r"
        function nestedLoop(int $count): int {
            $result = 0;
            for ($a = 0; $a < $count; $a++) {
                for ($b = 0; $b < $count; $b++) {
                    for ($c = 0; $c < $count; $c++) {
                        for ($d = 0; $d < $count; $d++) {
                            for ($e = 0; $e < $count; $e++) {
                                for ($f = 0; $f < $count; $f++) {
                                    $result++;
                                }
                            }
                        }
                    }
                }
            }
            return $result;
        }
        ",
        OptimizationConfiguration::default(),
    );
    let chunk = &unit.functions[0].chunk;
    assert_eq!(chunk.register_count, 8);
    assert!(!chunk.code.iter().any(|instruction| matches!(
        instruction,
        Instruction::Move { .. } | Instruction::MoveOwned { .. }
    )));
}

#[test]
fn neutral_integer_arithmetic_is_removed() {
    let unit = compile(
        r"
        function normalize(int $value): int {
            return (($value + 0) * 1) - 0;
        }
        ",
        OptimizationConfiguration::default(),
    );
    let normalize = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"normalize")
        .expect("the function exists");

    assert!(!normalize.chunk.code.iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::AddImmediate { .. }
                | Instruction::SubtractImmediate { .. }
                | Instruction::IntMultiplyImmediate { .. }
        )
    }));
}

#[test]
fn nonescaping_promoted_scalar_objects_are_replaced() {
    let unit = compile(
        r"
        final readonly class Point {
            public function __construct(
                public int $x,
                public int $y,
            ) {}
        }

        function sum(int $x, int $y): int {
            $point = new Point($x, $y);
            return $point->x + $point->y;
        }
        ",
        OptimizationConfiguration::default(),
    );
    let sum = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"sum")
        .expect("the function exists");

    assert!(
        !sum.chunk.code.iter().any(|instruction| {
            matches!(
                instruction,
                Instruction::NewStatic { .. }
                    | Instruction::InitializeProperties { .. }
                    | Instruction::PropertyGetUnchecked { .. }
            )
        }),
        "code={:?}, descriptors={:?}, property_initializers={:?}, mask={}",
        sum.chunk.code,
        sum.chunk.ic_descriptors,
        sum.chunk.property_initialization_descriptors,
        sum.chunk.reference_register_mask,
    );
}

#[test]
fn nonescaping_static_dict_reads_are_replaced() {
    let unit = compile(
        r"
        function select(int $value): int {
            $values = dict['wanted' => $value, 'other' => 0];
            return $values['wanted'];
        }
        ",
        OptimizationConfiguration::default(),
    );
    let select = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"select")
        .expect("the function exists");

    assert!(
        !select.chunk.code.iter().any(|instruction| {
            matches!(
                instruction,
                Instruction::NewDict { .. }
                    | Instruction::IndexGet { .. }
                    | Instruction::DictIndexGetIntKey { .. }
                    | Instruction::DictIndexGetStringKey { .. }
            )
        }),
        "code={:?}, mask={}",
        select.chunk.code,
        select.chunk.reference_register_mask,
    );
}

#[test]
fn tuple_index_specialization_requires_proven_bounds() {
    let unit = compile(
        r"
        function exact((int, string) $values): int {
            return $values[0] + length!($values[1]);
        }

        function prefix((int, string, ...bool) $values): int {
            return $values[0] + length!($values[1]);
        }

        function beyond((int, string, ...bool) $values): mixed {
            return $values[2];
        }

        function dynamic((int, ...string) $values, int $index): mixed {
            return $values[$index];
        }
        ",
        OptimizationConfiguration::default(),
    );

    for (name, element_gets, index_gets) in [
        (&b"exact"[..], 2, 0),
        (&b"prefix"[..], 2, 0),
        (&b"beyond"[..], 0, 1),
        (&b"dynamic"[..], 0, 1),
    ] {
        let function = unit
            .functions
            .iter()
            .find(|function| function.name.as_bytes() == name)
            .expect("the function exists");
        let code = &function.chunk.code;
        assert_eq!(
            code.iter()
                .filter(|instruction| matches!(instruction, Instruction::ElementGet { .. }))
                .count(),
            element_gets,
            "{code:#?}",
        );
        assert_eq!(
            code.iter()
                .filter(|instruction| matches!(instruction, Instruction::IndexGet { .. }))
                .count(),
            index_gets,
            "{code:#?}",
        );
    }
}

#[test]
fn mixed_dictionary_keys_remain_generic() {
    let unit = compile(
        r#"
        type NonEmptyString = string & !"";
        type NonNegativeInt = 0..;

        final readonly class Captures {
            public function __construct(
                public dict<int|string, null|string> $values,
            ) {}

            public function capture(NonNegativeInt|NonEmptyString $key): null|string {
                if (!contains_key!($this->values, $key)) {
                    return null;
                }

                return $this->values[$key];
            }
        }
        "#,
        OptimizationConfiguration::default(),
    );
    let code = method(&unit, b"Captures::capture");

    assert!(
        code.iter()
            .any(|instruction| matches!(instruction, Instruction::IndexGet { .. })),
        "{code:#?}",
    );
    assert!(
        !code.iter().any(|instruction| matches!(
            instruction,
            Instruction::DictIndexGetIntKey { .. } | Instruction::DictIndexGetStringKey { .. }
        )),
        "{code:#?}",
    );
}

#[test]
fn unresolved_named_dictionary_keys_remain_generic() {
    let unit = compile(
        r"
        final readonly class Captures {
            public function __construct(
                public dict<int|string, null|string> $values,
            ) {}

            public function capture(FutureInt|FutureString $key): null|string {
                return $this->values[$key];
            }
        }
        ",
        OptimizationConfiguration::default(),
    );
    let code = method(&unit, b"Captures::capture");

    assert!(
        code.iter()
            .any(|instruction| matches!(instruction, Instruction::IndexGet { .. })),
        "{code:#?}",
    );
    assert!(
        !code.iter().any(|instruction| matches!(
            instruction,
            Instruction::DictIndexGetIntKey { .. } | Instruction::DictIndexGetStringKey { .. }
        )),
        "{code:#?}",
    );
}

#[test]
fn match_binding_expands_array_aliases() {
    let unit = compile(
        r"
        type Row = vec<mixed>;

        function fetch(): null|Row {
            return vec[0];
        }

        $row = fetch();
        $value = match ($row) {
            null => 0,
            $found @ Row => $found[0] as int,
        };
        ",
        OptimizationConfiguration::default(),
    );

    assert!(
        unit.main
            .code
            .iter()
            .any(|instruction| { matches!(instruction, Instruction::VecIndexGet { .. }) }),
        "{:#?}",
        unit.main.code
    );
    assert!(
        !unit
            .main
            .code
            .iter()
            .any(|instruction| { matches!(instruction, Instruction::StringIndexGet { .. }) }),
        "{:#?}",
        unit.main.code
    );
}

#[test]
fn immediately_called_noncapturing_closures_are_inlined() {
    let unit = compile(
        r"
        function double(int $value): int {
            $closure = fn(int $input): int => $input * 2;
            return $closure($value);
        }
        ",
        OptimizationConfiguration::default(),
    );
    let double = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"double")
        .expect("the function exists");

    assert!(!double.chunk.code.iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::MakeClosure { .. } | Instruction::CallValueUnchecked { .. }
        )
    }));
}

#[test]
fn reused_noncapturing_closures_remain_objects() {
    let unit = compile(
        r"
        function double_twice(int $value): int {
            $closure = fn(int $input): int => $input * 2;
            $value = $closure($value);
            return $closure($value);
        }
        ",
        OptimizationConfiguration::default(),
    );
    let double = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"double_twice")
        .expect("the function exists");

    assert!(
        double
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::MakeClosure { .. }))
    );
}

#[test]
fn scalar_this_properties_do_not_widen_the_frame_ownership_mask() {
    let unit = compile(
        r"
        final class Counter {
            public int $value = 0;

            public function increment(): static {
                $this->value = $this->value + 1;
                return $this;
            }
        }

        $counter = new Counter();
        $counter->increment();
        ",
        OptimizationConfiguration::default(),
    );

    let method = unit.classes[0]
        .methods
        .iter()
        .find(|method| method.name.as_bytes() == b"increment")
        .expect("the increment method exists");
    assert_eq!(method.function.chunk.reference_register_mask, 0);
}

#[test]
fn optimizer_fuses_string_constants_into_concatenation() {
    let unit = compile(
        r#"
        function decorate(string $value): string {
            return $value . ": " . $value . ".";
        }
        "#,
        OptimizationConfiguration::default(),
    );
    let code = &unit.functions[0].chunk.code;

    let fused = code
        .iter()
        .filter(|instruction| matches!(instruction, Instruction::ConcatenateRightConstant { .. }))
        .count();
    assert_eq!(fused, 2, "{code:#?}");
    assert!(code.iter().any(|instruction| matches!(
        instruction,
        Instruction::ConcatenateRightConstant {
            destination,
            source,
            ..
        } if destination == source
    )));
}

#[test]
fn optimizer_fuses_string_constants_on_both_sides() {
    let unit = compile(
        r#"
        final readonly class Person {
            public function __construct(public string $name, public int $age) {}
        }

        $person = new Person("Ada", 36);
        write_line!("Hi!" . $person->name . " is " . $person->age . ".");
        "#,
        OptimizationConfiguration::default(),
    );
    let code = &unit.main.code;

    assert_eq!(
        code.iter()
            .filter(|instruction| matches!(
                instruction,
                Instruction::ConcatenateRightConstant { .. }
            ))
            .count(),
        2,
        "{code:#?}"
    );
    let (index, source, constant) = code
        .iter()
        .enumerate()
        .find_map(|(index, instruction)| match instruction {
            Instruction::ConcatenateLeftConstant {
                source, constant, ..
            } => Some((index, *source, *constant)),
            _ => None,
        })
        .expect("the leading string constant is fused");
    assert!(matches!(
        code[index - 1],
        Instruction::PropertyGetUnchecked { destination, .. } if destination == source
    ));
    assert!(matches!(
        &unit.main.constants[usize::from(constant.index())],
        Literal::String(value) if value.as_bytes() == b"Hi!"
    ));
    assert!(!code.iter().any(|instruction| matches!(
        instruction,
        Instruction::LoadConstant { constant: loaded, .. } if *loaded == constant
    )));
}

#[test]
fn abstract_class_methods_specialize_their_own_property_slots() {
    let unit = compile(
        r#"
        final class Target {
            public function take(int $value): int {
                return $value;
            }
        }

        abstract class Base {
            public function __construct(private Target $target) {}

            public function take(int $value): int {
                return $this->target->take($value);
            }
        }

        final class Child extends Base {}
        "#,
        OptimizationConfiguration::default(),
    );
    let code = method(&unit, b"Base::take");

    assert!(
        code.iter()
            .any(|instruction| matches!(instruction, Instruction::PropertyGetUnchecked { .. }))
    );
    assert!(code.iter().any(|instruction| matches!(
        instruction,
        Instruction::CallMethodUnchecked { .. } | Instruction::CallMethodDirect { .. }
    )));
}

#[test]
fn catch_handlers_preserve_unmodified_parameter_types() {
    let unit = compile(
        r"
        final readonly class Settings {
            public function __construct(public int $limit) {}
        }

        final class Failure {}

        final class Runner {
            public function read(Settings $settings, int $value): int {
                try {
                    if ($value < 0) {
                        throw new Failure();
                    }
                } catch (Failure $_) {
                    return 0;
                }

                return $settings->limit;
            }
        }
        ",
        OptimizationConfiguration::default(),
    );
    let code = method(&unit, b"Runner::read");

    assert!(
        code.iter()
            .any(|instruction| { matches!(instruction, Instruction::PropertyGetUnchecked { .. }) })
    );
}

#[test]
fn catch_handlers_preserve_locals_initialized_before_the_protected_region() {
    let unit = compile(
        r"
        final readonly class Settings {
            public function __construct(public int $limit) {}
        }

        final class Failure {}

        final class Runner {
            public function __construct(private Settings $settings) {}

            public function read(int $value): int {
                $settings = $this->settings;
                try {
                    if ($value < 0) {
                        throw new Failure();
                    }
                } catch (Failure $_) {}

                return $settings->limit;
            }
        }
        ",
        OptimizationConfiguration::default(),
    );
    let code = method(&unit, b"Runner::read");

    assert!(
        code.iter()
            .any(|instruction| { matches!(instruction, Instruction::PropertyGetUnchecked { .. }) })
    );
}

#[test]
fn catch_back_edges_preserve_loop_invariant_local_types() {
    let unit = compile(
        r"
        use Whim\Marker\NeverInline;

        final readonly class Settings {
            public function __construct(public int $limit) {}
        }

        final class Failure {}

        #[NeverInline]
        function positive(1.. $value): int {
            return $value;
        }

        final class Runner {
            public function __construct(private Settings $settings) {}

            public function read(int $value): int {
                $settings = $this->settings;
                if ($value <= 0) {
                    return 0;
                }

                while (true) {
                    try {
                        if ($value < 0) {
                            throw new Failure();
                        }
                    } catch (Failure $_) {
                        continue;
                    }

                    $result = $settings->limit;
                    return $result + positive($value);
                }
            }

            public function modified(int $value): int {
                if ($value <= 0) {
                    return 0;
                }

                while (true) {
                    try {
                        if ($value < 0) {
                            throw new Failure();
                        }
                    } catch (Failure $_) {
                        $value = 0;
                        continue;
                    }

                    return positive($value);
                }
            }
        }
        ",
        OptimizationConfiguration::default(),
    );
    let code = method(&unit, b"Runner::read");

    assert!(
        code.iter()
            .any(|instruction| { matches!(instruction, Instruction::CallNamedUnchecked { .. }) })
    );
    assert!(
        method(&unit, b"Runner::modified")
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CallNamed { .. }))
    );
}

#[test]
fn null_guards_refine_copied_aliases() {
    let unit = compile(
        r"
        use Whim\Marker\NeverInline;

        #[NeverInline]
        function positive(1.. $value): int {
            return $value;
        }

        function read(null|int $maximum): int {
            $limit = $maximum ?? 8192;
            if ($limit <= 0) {
                return 0;
            }

            return positive($limit);
        }
        ",
        OptimizationConfiguration::default(),
    );
    let code = &unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"read")
        .expect("the read function exists")
        .chunk
        .code;

    assert!(
        code.iter()
            .any(|instruction| { matches!(instruction, Instruction::CallNamedUnchecked { .. }) })
    );
}

#[test]
fn nullable_final_objects_specialize_after_a_null_guard() {
    let unit = compile(
        r"
        final readonly class Result {
            public function __construct(public int $value) {}
        }

        function read(null|Result $result): int {
            if ($result == null) {
                return 0;
            }

            return $result->value;
        }
        ",
        OptimizationConfiguration::default(),
    );
    let code = &unit.functions[0].chunk.code;

    assert!(
        code.iter()
            .any(|instruction| { matches!(instruction, Instruction::PropertyGetUnchecked { .. }) })
    );
}

#[test]
fn foreach_preserves_vec_element_types() {
    let unit = compile(
        r"
        type Event = (int, string);

        function tag(vec<Event> $events): int {
            foreach ($events as $event) {
                return $event[0];
            }

            return 0;
        }

        function isData(vec<Event> $events): int {
            foreach ($events as $event) {
                if ($event[0] == 1) {
                    return 1;
                }
            }

            return 0;
        }
        ",
        OptimizationConfiguration::default(),
    );
    let tag = &unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"tag")
        .expect("the tag function exists")
        .chunk
        .code;
    let is_data = &unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"isData")
        .expect("the isData function exists")
        .chunk
        .code;

    assert!(
        tag.iter()
            .any(|instruction| matches!(instruction, Instruction::ReturnUnchecked { .. }))
    );
    assert!(
        is_data
            .iter()
            .any(|instruction| matches!(instruction, Instruction::IntJumpUnlessImmediate { .. }))
    );
}

#[test]
fn dictionary_key_normalization_preserves_newtype_return_checks() {
    let unit = compile(
        r"
        newtype Key = int;

        function invalid(): dict<Key, string> {
            $values = dict[];
            $values[Key(1)] = 'value';
            return $values;
        }

        function valid(): dict<int, string> {
            $values = dict[];
            $values[Key(1)] = 'value';
            return $values;
        }

        function generic_invalid<T: int>(T $key): dict<T, string> {
            $values = dict[];
            $values[$key] = 'value';
            return $values;
        }
        ",
        OptimizationConfiguration::default(),
    );
    let invalid_function = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"invalid")
        .expect("the invalid function exists");
    let invalid = &invalid_function.chunk.code;
    let valid = &unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"valid")
        .expect("the valid function exists")
        .chunk
        .code;

    assert!(
        invalid
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Return { .. })),
        "{invalid:#?}",
    );
    let generic_invalid = &unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"generic_invalid")
        .expect("the generic invalid function exists")
        .chunk
        .code;
    assert!(
        generic_invalid
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Return { .. })),
        "{generic_invalid:#?}",
    );
    assert!(
        valid
            .iter()
            .any(|instruction| matches!(instruction, Instruction::ReturnReferenceUnchecked { .. })),
        "{valid:#?}"
    );
}

#[test]
fn mutable_array_masks_do_not_prove_literal_members() {
    let unit = compile(
        r"
        final class ZeroKeyDictionary {
            public dict<0, string> $values = dict[];

            public function fill(): void {
                $index = 0;
                while ($index <= 2) {
                    $this->values[$index] = 'filled';
                    $index++;
                }
            }
        }

        final class LiteralCounterBag {
            public dict<int, 1> $values = dict[0 => 1];

            public function increment(): void {
                $this->values[0]++;
            }
        }
        ",
        OptimizationConfiguration::default(),
    );

    assert!(
        method(&unit, b"ZeroKeyDictionary::fill")
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PropertyIndexSet { .. }))
    );
    assert!(
        method(&unit, b"LiteralCounterBag::increment")
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PropertyIndexUpdate { .. }))
    );
}

#[test]
fn type_test_results_do_not_inherit_the_tested_type() {
    let unit = compile(
        r"
        function false_is_not_true(): bool {
            return !(false is true);
        }

        function int_is_not_true(): bool {
            return !(1 is true);
        }
        ",
        OptimizationConfiguration::default(),
    );
    for name in [b"false_is_not_true".as_slice(), b"int_is_not_true"] {
        let function = unit
            .functions
            .iter()
            .find(|function| function.name.as_bytes() == name)
            .expect("the function exists");
        assert!(
            !function.chunk.code.windows(2).any(|instructions| {
                matches!(
                    instructions,
                    [
                        Instruction::LoadFalse { destination },
                        Instruction::ReturnScalarUnchecked { source },
                    ] if destination == source
                )
            }),
            "{:#?}",
            function.chunk.code,
        );
    }
}

#[test]
fn discarded_property_removals_mutate_the_property_in_place() {
    let unit = compile(
        r"
        final class Registry {
            private dict<string, int> $values = dict[];

            public function remove(string $key): void {
                if (!contains_key!($this->values, $key)) {
                    return;
                }

                $values = $this->values;
                remove!($values, $key);
                $this->values = $values;
            }
        }
        ",
        OptimizationConfiguration::default(),
    );

    assert!(
        method(&unit, b"Registry::remove")
            .iter()
            .any(|instruction| {
                matches!(
                    instruction,
                    Instruction::PropertyIndexUpdateUnchecked {
                        mode: PropertyIndexUpdateMode::Remove,
                        ..
                    }
                )
            })
    );
}

#[test]
fn specialized_float_constants_continue_folding() {
    let unit = compile(
        r"
        function folded(): float {
            $width = 50;
            $radius = 0.7;
            $step = 2.0 * $radius / $width;
            return $step;
        }
        ",
        OptimizationConfiguration::default(),
    );
    let Some(function) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"folded")
    else {
        panic!("the folded function exists");
    };

    assert!(function.chunk.code.iter().all(|instruction| {
        !matches!(
            instruction,
            Instruction::Multiply { .. }
                | Instruction::FloatMultiply { .. }
                | Instruction::FloatMultiplyConstant { .. }
                | Instruction::Divide { .. }
        )
    }));
}

#[test]
fn multiplying_a_float_by_two_uses_addition() {
    let unit = compile(
        r"
        function double(float $value): float {
            return $value * 2.0;
        }
        ",
        OptimizationConfiguration::default(),
    );
    let Some(function) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"double")
    else {
        panic!("the double function exists");
    };

    assert!(function.chunk.code.iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::FloatAdd { left, right, .. } if left == right
        )
    }));
    assert!(function.chunk.code.iter().all(|instruction| {
        !matches!(
            instruction,
            Instruction::FloatMultiply { .. } | Instruction::FloatMultiplyConstant { .. }
        )
    }));
}

#[test]
fn dependent_float_squares_remain_sequential() {
    let unit = compile(
        r"
        function fourth(float $value): float {
            $value = $value * $value;
            $result = $value * $value;
            return $result;
        }
        ",
        OptimizationConfiguration::default(),
    );
    let Some(function) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"fourth")
    else {
        panic!("the fourth function exists");
    };

    assert!(
        function
            .chunk
            .code
            .iter()
            .all(|instruction| !matches!(instruction, Instruction::FloatSquares { .. }))
    );
}

#[test]
fn loop_carried_square_sums_rotate_into_the_header() {
    let unit = compile(
        r"
        function orbit(float $real, float $imaginary): float {
            $remaining = 1000;
            $realSquared = $real * $real;
            $imaginarySquared = $imaginary * $imaginary;
            while (
                $realSquared + $imaginarySquared < 1000000.0
                && $remaining > 0
            ) {
                $imaginary = $real * $imaginary * 2.0;
                $real = $realSquared - $imaginarySquared;
                $realSquared = $real * $real;
                $imaginarySquared = $imaginary * $imaginary;
                $remaining--;
            }

            return $real;
        }
        ",
        OptimizationConfiguration::default(),
    );
    let Some(function) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"orbit")
    else {
        panic!("the orbit function exists");
    };

    assert!(
        function
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::FloatSquaresSumBranch { .. }))
    );
    assert!(
        function
            .chunk
            .code
            .iter()
            .all(|instruction| !matches!(instruction, Instruction::FloatSquares { .. }))
    );
}

#[test]
fn parameter_check_elision_is_configurable() {
    let unit = compile(
        CALL_SOURCE,
        OptimizationConfiguration {
            elide_parameter_checks: false,
            ..OptimizationConfiguration::default()
        },
    );

    assert!(
        method(&unit, b"Caller::proven")
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CallMethod { .. }))
    );
    assert!(method(&unit, b"Caller::proven").iter().all(|instruction| {
        !matches!(
            instruction,
            Instruction::CallMethodUnchecked { .. } | Instruction::CallMethodDirect { .. }
        )
    }));
}

#[test]
fn exact_calls_with_omitted_defaults_elide_parameter_checks() {
    let unit = compile(
        r#"
        final class Target {
            public function take(int $value, null|string $label = null): int {
                return $value;
            }
        }

        function call(Target $target, int $value): int {
            return $target->take($value);
        }
        "#,
        OptimizationConfiguration::default(),
    );
    let Some(call) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"call")
    else {
        panic!("the caller exists");
    };

    assert!(call.chunk.code.iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::CallMethodUnchecked { .. } | Instruction::CallMethodDirect { .. }
        )
    }));
}

#[test]
fn fresh_array_arguments_prove_alias_intersections() {
    let unit = compile(
        r#"
        type NonEmptyString = string & !"";
        type Field = (NonEmptyString, string);
        type Fields = array<_, Field>;

        final class FieldMap {
            #[Whim\Marker\NeverInline]
            public function __construct(Fields $fields) {}
        }

        function make(string $value): FieldMap {
            return new FieldMap(vec[("Content-Length", $value)]);
        }
        "#,
        OptimizationConfiguration::default(),
    );
    let Some(make) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"make")
    else {
        panic!("the factory exists");
    };

    assert!(
        make.chunk.code.iter().any(|instruction| {
            matches!(
                instruction,
                Instruction::CallMethodUnchecked { .. } | Instruction::CallMethodDirect { .. }
            )
        }),
        "{:#?}",
        make.chunk.code,
    );
    assert!(
        make.chunk
            .code
            .iter()
            .all(|instruction| !matches!(instruction, Instruction::CallMethod { .. }))
    );
}

#[test]
fn constructor_arguments_proven_through_control_flow_elide_parameter_checks() {
    let unit = compile(
        r#"
        final readonly class Head {
            #[Whim\Marker\NeverInline]
            public function __construct(
                public string $method,
                public null|int $length,
                public bool $chunked,
                public null|string $host,
            ) {}
        }

        function parse(string $method, null|string $host, int $count): Head {
            $length = null;
            if ($count > 0) {
                $length = $count;
            }

            $chunked = $count != 0;
            return new Head($method, $length, $chunked, $host);
        }
        "#,
        OptimizationConfiguration::default(),
    );
    let Some(parse) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"parse")
    else {
        panic!("the parser exists");
    };

    assert!(
        parse.chunk.code.iter().any(|instruction| {
            matches!(
                instruction,
                Instruction::CallMethodUnchecked { .. } | Instruction::CallMethodDirect { .. }
            )
        }),
        "{:#?}",
        parse.chunk.code,
    );
    assert!(
        parse
            .chunk
            .code
            .iter()
            .all(|instruction| !matches!(instruction, Instruction::CallMethod { .. }))
    );
}

#[test]
fn merged_enum_cases_elide_constructor_parameter_checks() {
    let unit = compile(
        r"
        enum Version {
            case One;
            case Two;
        }

        final readonly class Head {
            public function __construct(public Version $version) {}
        }

        function make(string $source): Head {
            $version = match ($source) {
                'one' => Version::One,
                $_ => Version::Two,
            };
            return new Head($version);
        }
        ",
        OptimizationConfiguration::default(),
    );
    let make = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"make")
        .expect("the factory exists");

    assert!(
        !make
            .chunk
            .code
            .iter()
            .any(|instruction| { matches!(instruction, Instruction::CallMethod { .. }) })
    );
}

#[test]
fn typed_call_arrays_refine_foreach_values() {
    let unit = compile(
        r"
        use Whim\Marker\NeverInline;

        #[NeverInline]
        function tokens(): vec<string> {
            return vec['upgrade'];
        }

        final readonly class Head {
            public function __construct(public null|string $upgrade) {}
        }

        function make(bool $select): Head {
            $upgrade = null;
            if ($select) {
                foreach (tokens() as $token) {
                    $upgrade = $token;
                    break;
                }
            }
            return new Head($upgrade);
        }
        ",
        OptimizationConfiguration::default(),
    );
    let make = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"make")
        .expect("the factory exists");

    assert!(
        !make
            .chunk
            .code
            .iter()
            .any(|instruction| { matches!(instruction, Instruction::CallMethod { .. }) })
    );
}

#[test]
fn named_calls_with_omitted_defaults_elide_parameter_checks() {
    let unit = compile(
        r#"
        function target(int $value, null|string $label = null): int {
            return $value;
        }

        function call(int $value): int {
            return target($value);
        }
        "#,
        OptimizationConfiguration::default(),
    );
    let Some(call) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"call")
    else {
        panic!("the caller exists");
    };

    assert!(call.chunk.code.iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::CallNamedUnchecked { .. }
                | Instruction::CallNamedConstantUnchecked { .. }
                | Instruction::CallSelfUnchecked { .. }
        )
    }));
}

#[test]
fn string_length_ranges_elide_only_proven_type_checks() {
    let unit = compile(
        r#"
        use Whim\Marker\NeverInline;

        #[NeverInline]
        function exact(): string[4] {
            return "four";
        }

        #[NeverInline]
        function minimum(): string[4..] {
            return "four";
        }

        #[NeverInline]
        function maximum(): string[..=4] {
            return "four";
        }

        #[NeverInline]
        function bounded(): string[2..=4] {
            return "four";
        }

        #[NeverInline]
        function empty(): string[0] {
            return "";
        }

        #[NeverInline]
        function covered(): string[1..=8] {
            return "four";
        }

        #[NeverInline]
        function intersected(): string[1..=8]&string[4..=12] {
            return "four";
        }

        #[NeverInline]
        function except_four(): string&!string[4] {
            return "five!";
        }

        function exact_to_minimum(): string[1..] {
            return exact();
        }

        function exact_to_maximum(): string[..=8] {
            return exact();
        }

        function exact_to_bounded(): string[1..=8] {
            return exact();
        }

        function minimum_to_minimum(): string[1..] {
            return minimum();
        }

        function maximum_to_maximum(): string[..=8] {
            return maximum();
        }

        function bounded_to_minimum(): string[1..] {
            return bounded();
        }

        function bounded_to_maximum(): string[..=8] {
            return bounded();
        }

        function bounded_to_bounded(): string[1..=8] {
            return bounded();
        }

        function empty_to_maximum(): string[..=8] {
            return empty();
        }

        function covered_by_union(): string[1..=4]|string[5..=8] {
            return covered();
        }

        function narrowed_by_intersection(): string[4..=8] {
            return intersected();
        }

        function covered_by_complement(): string[..=3]|string[5..] {
            return except_four();
        }

        type Four = string[4];
        type NonEmpty = string[1..];

        #[NeverInline]
        function alias_exact(): Four {
            return "four";
        }

        function alias_to_alias(): NonEmpty {
            return alias_exact();
        }

        #[NeverInline]
        function identity<T>(T $value): T {
            return $value;
        }

        function generic_to_minimum(): string[1..] {
            return identity::<string[4]>("four");
        }

        #[NeverInline]
        function consume(string[1..=8] $_): void {}

        function pass_exact(): void {
            consume(exact());
        }

        #[NeverInline]
        function consume_exact(string[4] $_): void {}

        function pass_covered(): void {
            consume_exact(covered());
        }

        final class LengthBox {
            public string[1..=8] $value = "a";
            public string[4] $exact = "four";

            public function set(): void {
                $this->value = exact();
            }

            public function setUnproven(string[1..=8] $value): void {
                $this->exact = $value;
            }
        }

        function uncovered_gap(): string[1..=3]|string[5..=8] {
            return covered();
        }

        function narrower_than_source(): string[4] {
            return covered();
        }
        "#,
        OptimizationConfiguration::default(),
    );

    for name in [
        b"exact_to_minimum".as_slice(),
        b"exact_to_maximum".as_slice(),
        b"exact_to_bounded".as_slice(),
        b"minimum_to_minimum".as_slice(),
        b"maximum_to_maximum".as_slice(),
        b"bounded_to_minimum".as_slice(),
        b"bounded_to_maximum".as_slice(),
        b"bounded_to_bounded".as_slice(),
        b"empty_to_maximum".as_slice(),
        b"covered_by_union".as_slice(),
        b"narrowed_by_intersection".as_slice(),
        b"covered_by_complement".as_slice(),
        b"alias_to_alias".as_slice(),
        b"generic_to_minimum".as_slice(),
    ] {
        let function = unit
            .functions
            .iter()
            .find(|function| function.name.as_bytes() == name)
            .expect("the checked function exists");
        assert!(
            function.chunk.code.iter().any(|instruction| matches!(
                instruction,
                Instruction::ReturnUnchecked { .. }
                    | Instruction::ReturnReferenceUnchecked { .. }
                    | Instruction::ReturnScalarUnchecked { .. }
            )),
            "{}: {:#?}",
            String::from_utf8_lossy(name),
            function.chunk.code,
        );
        assert!(
            function
                .chunk
                .code
                .iter()
                .all(|instruction| !matches!(instruction, Instruction::Return { .. })),
            "{}: {:#?}",
            String::from_utf8_lossy(name),
            function.chunk.code,
        );
    }

    let pass_exact = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"pass_exact")
        .expect("the caller exists");
    assert!(pass_exact.chunk.code.iter().any(|instruction| matches!(
        instruction,
        Instruction::CallNamedUnchecked { .. }
            | Instruction::CallNamedConstantUnchecked { .. }
            | Instruction::CallSelfUnchecked { .. }
    )));
    assert!(
        pass_exact
            .chunk
            .code
            .iter()
            .all(|instruction| !matches!(instruction, Instruction::CallNamed { .. }))
    );
    assert!(
        method(&unit, b"LengthBox::set")
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PropertySetUnchecked { .. }))
    );
    let pass_covered = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"pass_covered")
        .expect("the checked caller exists");
    assert!(
        pass_covered
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CallNamed { .. }))
    );
    assert!(
        method(&unit, b"LengthBox::setUnproven")
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PropertySet { .. }))
    );

    for name in [b"uncovered_gap".as_slice(), b"narrower_than_source"] {
        let function = unit
            .functions
            .iter()
            .find(|function| function.name.as_bytes() == name)
            .expect("the checked function exists");
        assert!(
            function
                .chunk
                .code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Return { .. })),
            "{}: {:#?}",
            String::from_utf8_lossy(name),
            function.chunk.code,
        );
    }
}

#[test]
fn foreach_tuple_elements_elide_named_call_parameter_checks() {
    let unit = compile(
        r#"
        type Name = string&!'';
        type Field = (Name, string);
        type Fields = vec<Field>|dict<int, Field>;

        #[Whim\Marker\NeverInline]
        function valid(string $value): bool {
            return $value != '';
        }

        function validate(Fields $fields): bool {
            foreach ($fields as ($name, $value)) {
                if (valid($name) && valid($value)) {
                    return true;
                }
            }

            return false;
        }
        "#,
        OptimizationConfiguration::default(),
    );
    let Some(validate) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"validate")
    else {
        panic!("the validator exists");
    };

    assert!(
        validate
            .chunk
            .code
            .iter()
            .any(|instruction| { matches!(instruction, Instruction::CallNamedUnchecked { .. }) }),
        "{:#?}",
        validate.chunk.code,
    );
    assert!(
        validate
            .chunk
            .code
            .iter()
            .all(|instruction| !matches!(instruction, Instruction::CallNamed { .. }))
    );
    assert!(
        validate
            .chunk
            .code
            .iter()
            .all(|instruction| !matches!(instruction, Instruction::CheckDestructure { .. }))
    );
}

#[test]
fn captured_parameter_types_elide_checks_inside_closures() {
    let unit = compile(
        r#"
        function target(int $value): int {
            return $value;
        }

        function capture(int $value): fn(): int {
            return fn(): int => target($value);
        }
        "#,
        OptimizationConfiguration {
            inline_leaf_calls: false,
            ..OptimizationConfiguration::default()
        },
    );
    let Some(closure) =
        unit.functions.iter().find(|function| {
            function.name.as_bytes() != b"target"
                && function.name.as_bytes() != b"capture"
                && function.chunk.code.iter().any(|instruction| {
                    matches!(instruction, Instruction::CallNamedUnchecked { .. })
                })
        })
    else {
        panic!("the closure call has no parameter check");
    };

    assert!(
        closure
            .chunk
            .code
            .iter()
            .all(|instruction| !matches!(instruction, Instruction::CallNamed { .. }))
    );
}

#[test]
fn nested_closures_propagate_capture_types_to_the_innermost_body() {
    let unit = compile(
        r#"
        function target(int $value): int {
            return $value;
        }

        function nested(int $value): int {
            return (
                fn(): int => (
                    fn(): int => (fn(): int => target($value))()
                )()
            )();
        }
        "#,
        OptimizationConfiguration {
            inline_leaf_calls: false,
            ..OptimizationConfiguration::default()
        },
    );
    let closures: Vec<_> = unit
        .functions
        .iter()
        .filter(|function| !matches!(function.name.as_bytes(), b"target" | b"nested"))
        .collect();

    assert_eq!(closures.len(), 3);
    assert!(closures.iter().all(|closure| {
        matches!(
            closure.capture_types.as_slice(),
            [Some(TypeDescriptor::Int)]
        )
    }));
    assert!(closures.iter().any(|closure| {
        closure
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CallNamedUnchecked { .. }))
    }));
}

#[test]
fn interface_methods_with_omitted_defaults_elide_checks_inside_closures() {
    let unit = compile(
        r#"
        interface Reader {
            public function read(null|int $length = null): string;
            public function write(string $value, null|int $offset = null): void;
        }

        function use_reader(Reader $reader): fn(): null {
            return function () use ($reader): null {
                $value = $reader->read();
                $reader->write($value);
                return null;
            };
        }
        "#,
        OptimizationConfiguration {
            inline_leaf_calls: false,
            immutable_function_floor: usize::MAX,
            ..OptimizationConfiguration::default()
        },
    );
    let Some((position, closure)) = unit
        .functions
        .iter()
        .enumerate()
        .find(|(_, function)| function.name.as_bytes() != b"use_reader")
    else {
        panic!("the closure calls exist");
    };
    assert_eq!(closure.capture_types.len(), 1);
    assert!(
        closure
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CallMethod { .. }))
    );

    let units = [&unit.unit];
    let world = World::new(&units, &[]);
    let optimized = optimize_function(
        &unit.unit,
        position,
        Vec::new(),
        &world,
        &unit._heap,
        OptimizationConfiguration {
            inline_leaf_calls: false,
            ..OptimizationConfiguration::default()
        },
    )
    .0;

    assert_eq!(
        optimized
            .code
            .iter()
            .filter(|instruction| matches!(
                instruction,
                Instruction::CallMethodUnchecked { .. } | Instruction::CallMethodDirect { .. }
            ))
            .count(),
        2,
    );
    assert!(optimized.code.iter().all(|instruction| !matches!(
        instruction,
        Instruction::CallMethod { .. } | Instruction::CheckDiscardedResult { .. }
    )));
}

#[test]
fn interface_method_return_types_specialize_consumers() {
    let unit = compile(
        r#"
        final readonly class Response {
            public function __construct(public int $status) {}
        }

        interface Handler {
            public function handle(): Response;
        }

        function status(Handler $handler): int {
            return $handler->handle()->status;
        }
        "#,
        OptimizationConfiguration {
            inline_leaf_calls: false,
            ..OptimizationConfiguration::default()
        },
    );
    let Some(status) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"status")
    else {
        panic!("the status function exists");
    };

    assert!(
        status
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PropertyGetUnchecked { .. })),
        "{:#?}",
        status.chunk.code
    );
    assert!(
        status
            .chunk
            .code
            .iter()
            .all(|instruction| !matches!(instruction, Instruction::PropertyGet { .. }))
    );
}

#[test]
fn equivalent_final_class_origins_survive_control_flow_merges() {
    let unit = compile(
        r#"
        final readonly class Response {
            public function __construct(public int $status) {}
        }

        interface Handler {
            public function handle(): Response;
        }

        function status(Handler $handler, bool $fallback): int {
            $response = $handler->handle();
            if ($fallback) {
                $response = new Response(500);
            }

            return $response->status;
        }
        "#,
        OptimizationConfiguration {
            inline_leaf_calls: false,
            ..OptimizationConfiguration::default()
        },
    );
    let Some(status) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"status")
    else {
        panic!("the status function exists");
    };

    assert!(
        status
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PropertyGetUnchecked { .. }))
    );
    assert!(
        status
            .chunk
            .code
            .iter()
            .all(|instruction| !matches!(instruction, Instruction::PropertyGet { .. }))
    );
}

#[test]
fn non_null_branches_specialize_integer_arithmetic() {
    let unit = compile(
        r#"
        function increment(null|int $value): null|int {
            if ($value == null) {
                return null;
            }

            return $value + 1;
        }
        "#,
        OptimizationConfiguration::default(),
    );
    let Some(increment) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"increment")
    else {
        panic!("the increment function exists");
    };

    assert!(
        increment
            .chunk
            .code
            .iter()
            .all(|instruction| !matches!(instruction, Instruction::Add { .. }))
    );
}

#[test]
fn exact_call_results_only_request_reference_teardown_when_needed() {
    let unit = compile(
        r#"
        function number(): int {
            return 42;
        }

        function text(): string {
            return "answer";
        }

        $number = number();
        $text = text();
        "#,
        OptimizationConfiguration::default(),
    );

    for instruction in &unit.main.code {
        let Instruction::CallNamedUnchecked {
            destination, cache, ..
        } = instruction
        else {
            continue;
        };
        let name = match &unit.main.ic_descriptors[cache.index() as usize] {
            IcDescriptor::Member { name, .. } => name.as_bytes(),
            IcDescriptor::ClassMember { .. } => unreachable!(),
        };
        let owns_reference = unit.main.reference_register_mask & (1u64 << destination.index()) != 0;
        match name {
            b"number" => assert!(!owns_reference),
            b"text" => assert!(owns_reference),
            _ => unreachable!(),
        }
    }
}

#[test]
fn cold_block_layout_is_configurable() {
    let source = r"
        #[Whim\Marker\Cold]
        function reject(): never {
            throw new Whim\Unwind\Error('rejected');
        }

        function validate(int $value): int {
            if ($value == 0) {
                reject();
            }

            return $value + 1;
        }
    ";
    let unoptimized_layout = compile(
        source,
        OptimizationConfiguration {
            cold_block_layout: false,
            ..OptimizationConfiguration::default()
        },
    );
    let validate = unoptimized_layout
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"validate")
        .expect("the validator exists");
    let cold_call = validate
        .chunk
        .code
        .iter()
        .position(|instruction| {
            matches!(
                instruction,
                Instruction::CallNamed { .. }
                    | Instruction::CallNamedUnchecked { .. }
                    | Instruction::CallNamedDiscarded { .. }
            )
        })
        .expect("the cold call remains");
    let hot_return = validate
        .chunk
        .code
        .iter()
        .position(|instruction| {
            matches!(
                instruction,
                Instruction::Return { .. }
                    | Instruction::ReturnUnchecked { .. }
                    | Instruction::ReturnReferenceUnchecked { .. }
                    | Instruction::ReturnScalarUnchecked { .. }
                    | Instruction::ReturnIntUnchecked { .. }
            )
        })
        .expect("the hot path returns");
    assert!(cold_call < hot_return, "{:#?}", validate.chunk.code);
}

#[test]
fn a_scalar_iterator_step_needs_no_reference_teardown() {
    let unit = compile(
        r"
        final class IteratorStep {
            private int $current = 0;

            public function __construct(private int $limit) {}

            public function next(): null|(int, int) {
                if ($this->current >= $this->limit) {
                    return null;
                }

                $key = $this->current;
                $value = ($key * 17 + 11) % 1009;
                $this->current++;
                return ($key, $value);
            }
        }
        ",
        OptimizationConfiguration::default(),
    );

    let next = unit
        .classes
        .iter()
        .flat_map(|class| &class.methods)
        .find(|method| method.function.name.as_bytes() == b"IteratorStep::next")
        .expect("the next method exists");
    assert_eq!(next.function.chunk.reference_register_mask, 0);
}

#[test]
fn locally_built_string_int_dicts_fuse_compound_addition_precisely() {
    let unit = compile(
        r"
        function accumulate(int $count): int {
            $source = dict[];
            $destination = dict[];
            for ($index = 0; $index < $count; $index++) {
                $key = 'key-' . $index;
                $source[$key] = $index;
                $destination[$key] = 0;
            }

            foreach ($source as $key => $value) {
                $destination[$key] += $value;
            }

            return $destination['key-0'];
        }
        ",
        OptimizationConfiguration::default(),
    );

    let accumulate = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"accumulate")
        .expect("the accumulator exists");
    assert!(accumulate.chunk.code.iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::IndexAddAssign {
                mode: IndexAddMode::DictStringKeyIntValue,
                ..
            }
        )
    }));
}

#[test]
fn three_integer_operations_use_the_numeric_loop_executor() {
    let unit = compile(
        r"
        function checksum(int $count): int {
            $sum = 0;
            $index = 1;
            while ($index <= $count) {
                $sum = ($sum + $index * $index) % 2147483647;
                $index++;
            }

            return $sum;
        }
        ",
        OptimizationConfiguration::default(),
    );
    let checksum = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"checksum")
        .expect("the checksum function exists");

    assert!(
        checksum
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::IntNumericLoop { .. }))
    );
}

#[test]
fn doubled_float_updates_prepare_the_enclosing_numeric_loop() {
    let unit = compile(
        r"
        function orbit(float $real, float $imaginary): (int, float) {
            $x = 0.0;
            $y = 0.0;
            $iteration = 0;
            $magnitude = 0.0;
            while ($iteration < 360) {
                $x2 = $x * $x;
                $y2 = $y * $y;
                $magnitude = $x2 + $y2;
                if ($magnitude > 4.0) {
                    break;
                }

                $next = $x2 - $y2 + $real;
                $y = 2.0 * $x * $y + $imaginary;
                $x = $next;
                $iteration++;
            }

            return ($iteration, $magnitude);
        }
        ",
        OptimizationConfiguration::default(),
    );
    let Some(function) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"orbit")
    else {
        panic!("the orbit function exists");
    };

    assert!(
        function
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::FloatPairUpdate { .. }))
    );
    assert!(
        function.chunk.code.iter().any(|instruction| {
            matches!(instruction, Instruction::PreparedIntNumericLoop { .. })
        })
    );
}

#[test]
fn lowered_property_updates_fuse_before_operation_specialization() {
    let unit = compile(
        r"
        final class Stats {
            private dict<int, int> $counts = dict[0 => 0];
            private float $total = 0.0;

            public function record(int $key, float $value): void {
                $this->counts[$key]++;
                $this->total += $value;
            }
        }
        ",
        OptimizationConfiguration::default(),
    );
    let Some(record) = unit
        .classes
        .iter()
        .flat_map(|class| &class.methods)
        .find(|method| method.function.name.as_bytes() == b"Stats::record")
    else {
        panic!("the record method exists");
    };

    assert!(record.function.chunk.code.iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::PropertyIndexUpdateUnchecked { .. }
        )
    }));
    assert!(
        record
            .function
            .chunk
            .code
            .iter()
            .any(|instruction| { matches!(instruction, Instruction::PropertyAddUnchecked { .. }) })
    );
}

#[test]
fn direct_property_array_writes_mutate_in_place() {
    let source = r"
        final class Buffer {
            private vec<int> $values = vec[];
            private dict<string, int> $named = dict[];
            private int $offset = 0;

            public function push(int $value): void {
                $this->values[] = $value;
            }

            public function set(int $index, int $value): void {
                $this->values[$index] = $value;
            }

            public function setComputed(int $value): void {
                $index = $this->offset;
                if ($index < 0) {
                    write_line!($value);
                    return;
                }

                $this->values[$index] = $value;
            }

            public function setNamed(int $index, int $value): void {
                $this->named['key-' . $index] = $value;
            }

            public function setNamedWith(
                fn(): string $key,
                int $value,
            ): void {
                $this->named[$key()] = $value;
            }

            public function removeNamed(string $key): int {
                return remove!($this->named, $key);
            }

            public function swapRemove(int $index): int {
                return swap_remove!($this->values, $index);
            }

            public function removeFirst(): int {
                return remove_first!($this->values);
            }

            public function removeLast(): int {
                return remove_last!($this->values);
            }
        }
        ";
    let unoptimized = compile(
        source,
        OptimizationConfiguration {
            enabled: false,
            ..OptimizationConfiguration::default()
        },
    );
    assert!(
        method(&unoptimized, b"Buffer::push")
            .iter()
            .any(|instruction| {
                matches!(
                    instruction,
                    Instruction::PropertyIndexUpdate {
                        mode: PropertyIndexUpdateMode::Append,
                        ..
                    }
                )
            })
    );
    assert!(
        method(&unoptimized, b"Buffer::set")
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PropertyIndexSet { .. }))
    );
    for name in [
        b"Buffer::setNamed".as_slice(),
        b"Buffer::setNamedWith".as_slice(),
    ] {
        let code = method(&unoptimized, name);
        assert!(
            code.iter()
                .any(|instruction| matches!(instruction, Instruction::PropertyIndexSet { .. }))
        );
        assert!(!code.iter().any(|instruction| matches!(
            instruction,
            Instruction::PropertyGet { .. } | Instruction::PropertySet { .. }
        )));
    }

    for (name, mode) in [
        (b"Buffer::removeNamed".as_slice(), PropertyRemoveMode::Key),
        (b"Buffer::swapRemove".as_slice(), PropertyRemoveMode::Swap),
        (b"Buffer::removeFirst".as_slice(), PropertyRemoveMode::First),
        (b"Buffer::removeLast".as_slice(), PropertyRemoveMode::Last),
    ] {
        let code = method(&unoptimized, name);
        assert!(code.iter().any(|instruction| matches!(
            instruction,
            Instruction::PropertyRemove {
                mode: actual,
                ..
            } if *actual == mode
        )));
        assert!(!code.iter().any(|instruction| matches!(
            instruction,
            Instruction::PropertyGet { .. } | Instruction::PropertySet { .. }
        )));
    }

    let optimized = compile(source, OptimizationConfiguration::default());
    assert!(
        method(&optimized, b"Buffer::push")
            .iter()
            .any(|instruction| {
                matches!(
                    instruction,
                    Instruction::PropertyIndexUpdateUnchecked {
                        mode: PropertyIndexUpdateMode::Append,
                        ..
                    }
                )
            })
    );
    assert!(
        method(&optimized, b"Buffer::set")
            .iter()
            .any(|instruction| matches!(
                instruction,
                Instruction::PropertyIndexSetUnchecked { .. }
            ))
    );

    let computed = method(&optimized, b"Buffer::setComputed");
    let direct_write = computed
        .iter()
        .enumerate()
        .find_map(|(position, instruction)| match instruction {
            Instruction::PropertyIndexSetUnchecked { first_operand, .. } => {
                Some((position, *first_operand))
            }
            _ => None,
        });
    let Some((position, first_operand)) = direct_write else {
        panic!("setComputed has no direct indexed property write");
    };
    assert!(matches!(
        computed[position - 1],
        Instruction::Move {
            destination,
            source,
        } if destination.index() == first_operand.index() + 1 && source.index() == 1
    ));

    for name in [
        b"Buffer::setNamed".as_slice(),
        b"Buffer::setNamedWith".as_slice(),
    ] {
        let code = method(&optimized, name);
        assert!(code.iter().any(|instruction| matches!(
            instruction,
            Instruction::PropertyIndexSet { .. } | Instruction::PropertyIndexSetUnchecked { .. }
        )));
        assert!(!code.iter().any(|instruction| matches!(
            instruction,
            Instruction::PropertyGet { .. }
                | Instruction::PropertyGetUnchecked { .. }
                | Instruction::PropertySet { .. }
                | Instruction::PropertySetUnchecked { .. }
        )));
    }

    for (name, mode) in [
        (b"Buffer::removeNamed".as_slice(), PropertyRemoveMode::Key),
        (b"Buffer::swapRemove".as_slice(), PropertyRemoveMode::Swap),
        (b"Buffer::removeFirst".as_slice(), PropertyRemoveMode::First),
        (b"Buffer::removeLast".as_slice(), PropertyRemoveMode::Last),
    ] {
        assert!(method(&optimized, name).iter().any(|instruction| matches!(
            instruction,
            Instruction::PropertyRemoveUnchecked {
                mode: actual,
                ..
            } if *actual == mode
        )));
    }
}

#[test]
fn discarded_property_removals_keep_their_register_windows() {
    let source = r"
        final class Buffer {
            private dict<string, int> $named = dict[];
            private vec<int> $values = vec[];

            public function removeNamed(string $key): void {
                remove!($this->named, $key);
            }

            public function removeNamedGuarded(string $key): void {
                if (contains_key!($this->named, $key)) {
                    remove!($this->named, $key);
                }
            }

            public function swapRemove(int $index): void {
                swap_remove!($this->values, $index);
            }

            public function removeFirst(): void {
                remove_first!($this->values);
            }

            public function removeLast(): void {
                remove_last!($this->values);
            }
        }
        ";

    for optimization in [
        OptimizationConfiguration {
            enabled: false,
            ..OptimizationConfiguration::default()
        },
        OptimizationConfiguration::default(),
    ] {
        let unit = compile(source, optimization);
        let result = verify_unit(&unit);
        assert!(result.is_ok(), "{result:?}");
    }
}

#[test]
fn direct_property_writes_do_not_depend_on_prior_call_results() {
    let source = r"
        use Whim\Time\Instant;

        final class Holder {
            public dict<string, int> $entries = dict[];
        }

        $start = Instant::now();
        $first = Instant::now()->durationSince($start);
        $holder = new Holder();
        $index = 0;
        while ($index < 10_000) {
            $holder->entries['k' . $index] = $index;
            $index++;
        }
        ";
    let unoptimized = compile(
        source,
        OptimizationConfiguration {
            enabled: false,
            ..OptimizationConfiguration::default()
        },
    );
    assert!(
        unoptimized
            .main
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::PropertyIndexSet { .. }))
    );
    assert!(!unoptimized.main.code.iter().any(|instruction| matches!(
        instruction,
        Instruction::PropertyGet { .. } | Instruction::PropertySet { .. }
    )));

    let unit = compile(source, OptimizationConfiguration::default());
    assert!(
        unit.main.code.iter().any(|instruction| matches!(
            instruction,
            Instruction::PropertyIndexSetUnchecked { .. }
        ))
    );
    assert!(!unit.main.code.iter().any(|instruction| matches!(
        instruction,
        Instruction::PropertyGet { .. }
            | Instruction::PropertyGetUnchecked { .. }
            | Instruction::PropertySet { .. }
            | Instruction::PropertySetUnchecked { .. }
    )));
}

#[test]
fn dense_one_byte_string_matches_use_a_byte_table() {
    let unit = compile(
        r"
        function digit(string $value): int {
            return match ($value) {
                '0' => 0,
                '1' => 1,
                '2' => 2,
                '3' => 3,
                $_ => -1,
            };
        }
        ",
        OptimizationConfiguration::default(),
    );
    let Some(function) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"digit")
    else {
        panic!("the digit function exists");
    };
    let Some(table) = function.chunk.switch_tables.first() else {
        panic!("the digit match has a switch table");
    };

    assert!(matches!(
        table,
        SwitchTable::StringByte { base, targets, .. }
            if *base == b'0' && targets.len() == 4
    ));
}

#[test]
fn literal_matches_use_specialized_dispatch() {
    let unit = compile(
        r"
        function boolean(bool $value): int {
            return match ($value) {
                true => 1,
                false => 0,
            };
        }

        function integer(int $value): int {
            return match ($value) {
                0 => 1,
                1 => 2,
                2 => 3,
                3 => 4,
                $_ => 5,
            };
        }

        function floating(float $value): int {
            return match ($value) {
                0.0 => 1,
                1.0 => 2,
                2.0 => 3,
                3.0 => 4,
                $_ => 5,
            };
        }

        function ranged(int $value): int {
            return match ($value) {
                0..10 => 1,
                20..=30 => 2,
                42.. => 3,
                $_ => 4,
            };
        }

        function tupled(bool $left, bool $right): int {
            return match (($left, $right)) {
                (true, true) => 1,
                (true, false) => 2,
                (false, true) => 3,
                (false, false) => 4,
            };
        }

        function vectored(vec<bool> $value): int {
            return match ($value) {
                vec[true, true] => 1,
                vec[true, false] => 2,
                vec[false, true] => 3,
                vec[false, false] => 4,
                $_ => 5,
            };
        }

        function dictionary(dict<string, bool> $value): int {
            return match ($value) {
                dict['left' => true, 'right' => true] => 1,
                dict['left' => true, 'right' => false] => 2,
                dict['left' => false, 'right' => true] => 3,
                dict['left' => false, 'right' => false] => 4,
                $_ => 5,
            };
        }
        ",
        OptimizationConfiguration::default(),
    );
    let function = |name: &[u8]| {
        unit.functions
            .iter()
            .find(|function| function.name.as_bytes() == name)
            .expect("the function exists")
    };

    assert!(
        function(b"boolean")
            .chunk
            .code
            .iter()
            .any(|instruction| { matches!(instruction, Instruction::JumpIfFalse { .. }) })
    );
    assert!(
        function(b"integer").chunk.code.iter().any(|instruction| {
            matches!(instruction, Instruction::IntJumpUnlessImmediate { .. })
        })
    );
    assert!(
        function(b"ranged")
            .chunk
            .code
            .iter()
            .any(|instruction| { matches!(instruction, Instruction::IntRangeJumpUnless { .. }) })
    );
    assert!(
        function(b"tupled")
            .chunk
            .code
            .iter()
            .all(|instruction| { !matches!(instruction, Instruction::NewTuple { .. }) })
    );

    assert!(matches!(
        function(b"floating").chunk.switch_tables.first(),
        Some(SwitchTable::Float { .. })
    ));
    assert!(matches!(
        function(b"vectored").chunk.switch_tables.first(),
        Some(SwitchTable::Pattern { .. })
    ));
    assert!(matches!(
        function(b"dictionary").chunk.switch_tables.first(),
        Some(SwitchTable::DictionaryShape { .. })
    ));
}

#[test]
fn integer_range_matches_inline_into_callers() {
    let unit = compile(
        r"
        function classify(int $value): int {
            return match ($value) {
                0..10 => 1,
                20..=30 => 2,
                42.. => 3,
                $_ => 4,
            };
        }

        $sum = 0;
        $index = 0;
        while ($index < 100) {
            $sum += classify($index % 50);
            $index++;
        }
        ",
        OptimizationConfiguration::default(),
    );

    assert!(
        unit.main
            .code
            .iter()
            .any(|instruction| { matches!(instruction, Instruction::IntRangeJumpUnless { .. }) })
    );
    assert!(unit.main.code.iter().all(|instruction| {
        !matches!(
            instruction,
            Instruction::CallNamed { .. } | Instruction::CallNamedUnchecked { .. }
        )
    }));
}

#[test]
fn inlined_string_byte_loops_share_hoisted_properties_and_embed_literals() {
    let unit = compile(
        r#"
        use Whim\Marker\AlwaysInline;

        final class Cursor {
            public function __construct(
                private string $source,
                private int $position = 0,
            ) {}

            public function scan(): string {
                $value = '';
                while ($this->peek() != '"') {
                    $value .= $this->peek();
                    $this->position++;
                }

                return $value;
            }

            #[AlwaysInline]
            private function peek(): string {
                return $this->source[$this->position];
            }
        }
        "#,
        OptimizationConfiguration::default(),
    );
    let Some(scan) = unit
        .classes
        .iter()
        .flat_map(|class| &class.methods)
        .find(|method| method.function.name.as_bytes() == b"Cursor::scan")
    else {
        panic!("the scan method exists");
    };
    let quote_loads = scan
        .function
        .chunk
        .code
        .iter()
        .filter(|instruction| {
            let Instruction::LoadConstant { constant, .. } = instruction else {
                return false;
            };
            matches!(
                &scan.function.chunk.constants[usize::from(constant.index())],
                Literal::String(value) if value.as_bytes() == b"\""
            )
        })
        .count();
    let source_reads = scan
        .function
        .chunk
        .code
        .iter()
        .filter(|instruction| {
            matches!(
                instruction,
                Instruction::PropertyGetUnchecked { slot, .. } if slot.index() == 0
            )
        })
        .count();

    assert_eq!(quote_loads, 0);
    assert_eq!(source_reads, 1);
    assert!(scan.function.chunk.code.iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::StringByteJumpUnlessNotEqual { .. }
        )
    }));
}

#[test]
fn clear_of_a_runtime_trace_argument_is_preserved() {
    let unit = compile(
        r"
        final class Resource {}

        function consume(Resource $resource): void {
            drop!($resource);
        }
        ",
        OptimizationConfiguration::default(),
    );
    let Some(function) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"consume")
    else {
        panic!("the consume function exists");
    };
    let Some(trace) = function.chunk.trace_argument_registers.first().copied() else {
        panic!("the consumed parameter has a trace register");
    };

    assert!(function.chunk.code.iter().any(|instruction| {
        matches!(instruction, Instruction::Clear { target } if *target == trace)
    }));
}

#[test]
fn adjacent_promoted_properties_initialize_in_one_dispatch() {
    let unit = compile(
        r"
        final readonly class Record {
            public function __construct(
                public string $name,
                public int $index,
                public bool $active,
            ) {}
        }

        function make(string $name, int $index): Record {
            return new Record($name, $index, true);
        }
        ",
        OptimizationConfiguration::default(),
    );
    let Some(make) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"make")
    else {
        panic!("the factory exists");
    };
    let Some(descriptor) = make
        .chunk
        .code
        .iter()
        .find_map(|instruction| match instruction {
            Instruction::InitializeProperties { descriptor, .. } => Some(*descriptor),
            _ => None,
        })
    else {
        panic!("the promoted writes are fused");
    };
    assert!(
        !make
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::NewStatic { .. }))
    );

    assert_eq!(
        make.chunk
            .property_initialization_descriptor(descriptor)
            .entries
            .len(),
        3
    );
    assert!(
        make.chunk
            .property_initialization_descriptor(descriptor)
            .allocates
    );
}

#[test]
fn one_promoted_property_uses_regular_initialization() {
    let unit = compile(
        r"
        final readonly class Box {
            public function __construct(public int $value) {}
        }

        function make(int $value): Box {
            return new Box($value);
        }
        ",
        OptimizationConfiguration::default(),
    );
    let Some(make) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"make")
    else {
        panic!("the factory exists");
    };

    assert!(
        make.chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::NewStatic { .. }))
    );
    assert!(
        !make
            .chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::InitializeProperties { .. }))
    );
}

#[test]
fn computed_constructor_arguments_keep_allocation_and_batch_writes() {
    let unit = compile(
        r"
        final readonly class Record {
            public function __construct(
                public int $first,
                public int $second,
                public bool $active,
            ) {}
        }

        function make(int $value): Record {
            return new Record($value % 7, $value + 1, true);
        }
        ",
        OptimizationConfiguration::default(),
    );
    let Some(make) = unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == b"make")
    else {
        panic!("the factory exists");
    };
    let Some(descriptor) = make
        .chunk
        .code
        .iter()
        .find_map(|instruction| match instruction {
            Instruction::InitializeProperties { descriptor, .. } => Some(*descriptor),
            _ => None,
        })
    else {
        panic!("the promoted writes are fused");
    };

    assert!(
        make.chunk
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::NewStatic { .. }))
    );
    assert!(
        !make
            .chunk
            .property_initialization_descriptor(descriptor)
            .allocates
    );
}

#[test]
fn lazy_method_optimization_keeps_defaulted_constructor_call() {
    let unit = compile(
        r"
        final readonly class Wrapped {
            public function __construct(public null|int $value = null) {}

            public static function none(): Wrapped {
                return new Wrapped();
            }
        }
        ",
        OptimizationConfiguration {
            enabled: false,
            ..OptimizationConfiguration::default()
        },
    );
    let Some(class) = unit
        .classes
        .iter()
        .position(|class| class.name.as_bytes() == b"Wrapped")
    else {
        panic!("the class exists");
    };
    let Some(method) = unit.classes[class]
        .methods
        .iter()
        .position(|method| method.name.as_bytes() == b"none")
    else {
        panic!("the factory exists");
    };
    let world = World::new(&[], &[]);
    let optimized = optimize_method(
        &unit,
        class,
        method,
        vec![unit.classes[class].clone()],
        &world,
        &unit._heap,
        OptimizationConfiguration::default(),
    )
    .0;

    assert!(optimized.code.iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::CallMethod { .. }
                | Instruction::CallMethodUnchecked { .. }
                | Instruction::CallMethodDirect { .. }
        )
    }));
}
