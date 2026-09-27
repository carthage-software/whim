use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::AsMode;
use whim_bytecode::instruction::operands::JumpOffset;
use whim_bytecode::instruction::operands::Register;
use whim_bytecode::rewrite::relative_target;
use whim_bytecode::unit::CompiledFunction;
use whim_bytecode::unit::CompiledUnit;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;
use whim_optimizer::World;

use super::compile;
use super::optimize_function;

const FIXTURE: &str = include_str!("../../../../tests/_fixtures/default-parameter-checks.whim");

fn function<'a>(unit: &'a CompiledUnit, name: &str) -> &'a CompiledFunction {
    unit.functions
        .iter()
        .chain(
            unit.classes
                .iter()
                .flat_map(|class| class.methods.iter().map(|method| &method.function)),
        )
        .find(|function| function.name.as_bytes() == name.as_bytes())
        .unwrap()
}

fn skips_boundary_check(function: &CompiledFunction) -> bool {
    let code = &function.chunk.code;
    let Instruction::FillDefault { target, offset } = code[0] else {
        return false;
    };
    let destination = relative_target(0, offset.offset());
    destination.checked_sub(1).is_some_and(|index| {
        matches!(
            code[index],
            Instruction::AsCheck {
                destination,
                source,
                mode: AsMode::Boundary,
                ..
            } if destination == target && source == target
        )
    })
}

#[test]
fn supplied_defaults_skip_only_their_boundary_check() {
    let unit = compile(FIXTURE, OptimizationConfiguration::default());
    verify_unit(&unit).unwrap();
    for name in [
        "default_count",
        "invalid_integer",
        "optional_count",
        "invalid_count",
        "constrained",
        "Defaults::__construct",
        "Defaults::instance",
        "Defaults::static_method",
    ] {
        let function = function(&unit, name);
        assert!(
            skips_boundary_check(function),
            "{name}: {:?}",
            function.chunk.code
        );
        assert!(function.chunk.code.iter().any(|instruction| {
            matches!(
                instruction,
                Instruction::AsCheck {
                    mode: AsMode::Boundary,
                    ..
                }
            )
        }));
    }
}

#[test]
fn mutable_shapes_unknown_types_and_earlier_prologue_effects_keep_checks() {
    let unit = compile(FIXTURE, OptimizationConfiguration::default());
    verify_unit(&unit).unwrap();
    for name in [
        "mutable_shape",
        "mutable_shapes",
        "unknown_type",
        "tagged",
        "prior_default",
        "shape_after_default",
    ] {
        let function = function(&unit, name);
        assert!(
            !skips_boundary_check(function),
            "{name}: {:?}",
            function.chunk.code
        );
    }
}

#[test]
fn supplied_scalar_alias_defaults_skip_only_valid_pure_checks() {
    let unit = compile(
        r"
type NonZero<T: uint> = T & !0u;
type Hidden<T = Missing> = int;
function limit(null|NonZero<uint> $value = null): mixed { return $value; }
function wrong_bound(NonZero<int> $value = 1): mixed { return $value; }
function unknown_sibling(Missing|NonZero<uint> $value = 2u): mixed { return $value; }
function hidden_default(Hidden $value = 1): mixed { return $value; }
",
        OptimizationConfiguration::default(),
    );
    verify_unit(&unit).unwrap();
    assert!(skips_boundary_check(function(&unit, "limit")));
    for name in ["wrong_bound", "unknown_sibling", "hidden_default"] {
        let function = function(&unit, name);
        assert!(
            !skips_boundary_check(function),
            "{name}: {:?}",
            function.chunk.code
        );
    }
}

#[test]
fn supplied_defaults_keep_named_and_nested_resolution_checks() {
    let unit = compile(
        r"
final class Holder<out T> {}
final class Defaulted<out T, out U = Missing> {}
final class Bounded<out T: Missing|int> {}
type DefaultAlias<T = Missing> = int;
type BoundAlias<T: Missing|int> = T;
function supplied(Holder<Missing> $value = null): void {}
function defaulted(Defaulted<int> $value = null): void {}
function bounded(Bounded<int> $value = null): void {}
function default_alias(DefaultAlias $value = 1): void {}
function bound_alias(BoundAlias<int> $value = 1): void {}
function nested(dict['item' => Holder<Missing>] $value = null): void {}
function optional_callable(fn(): Missing $value = null): void {}
function optional_classname(classname<Missing> $value = null): void {}
",
        OptimizationConfiguration::default(),
    );
    verify_unit(&unit).unwrap();
    for name in [
        "supplied",
        "defaulted",
        "bounded",
        "default_alias",
        "bound_alias",
        "nested",
        "optional_callable",
        "optional_classname",
    ] {
        let function = function(&unit, name);
        assert!(
            !skips_boundary_check(function),
            "{name}: {:?}",
            function.chunk.code
        );
    }
}

#[test]
fn default_boundary_elision_respects_configuration() {
    let unit = compile(
        FIXTURE,
        OptimizationConfiguration {
            elide_type_checks: false,
            ..OptimizationConfiguration::default()
        },
    );
    verify_unit(&unit).unwrap();
    for name in [
        "optional_count",
        "default_count",
        "tagged",
        "Defaults::instance",
    ] {
        let function = function(&unit, name);
        assert!(
            !skips_boundary_check(function),
            "{name}: {:?}",
            function.chunk.code
        );
    }
}

#[test]
fn changed_boundary_checks_and_reentered_prologues_keep_checks() {
    for case in [
        "cast",
        "descriptor",
        "source",
        "destination",
        "required",
        "reentry",
    ] {
        let mut unit = compile(
            "function value(int $value = 1): int { return $value; }",
            OptimizationConfiguration {
                enabled: false,
                ..OptimizationConfiguration::default()
            },
        );
        let function = &mut unit.unit.functions[0];
        let Instruction::FillDefault { offset, .. } = function.chunk.code[0] else {
            panic!("the default starts the function");
        };
        let check = relative_target(0, offset.offset());
        let spare = Register::new(function.chunk.register_count);
        match case {
            "cast" | "descriptor" | "source" | "destination" => {
                let Instruction::AsCheck {
                    destination,
                    source,
                    descriptor,
                    mode,
                } = &mut function.chunk.code[check]
                else {
                    panic!("the supplied argument branches to its check");
                };
                match case {
                    "cast" => *mode = AsMode::Cast,
                    "descriptor" => {
                        function.chunk.type_descriptors[usize::from(descriptor.index())] =
                            TypeDescriptor::String;
                    }
                    "source" => *source = spare,
                    "destination" => *destination = spare,
                    _ => unreachable!(),
                }
                if matches!(case, "source" | "destination") {
                    function.chunk.register_count += 1;
                    function.chunk.local_register_count += 1;
                }
            }
            "required" => function.parameters[0].has_default = false,
            "reentry" => {
                let branch = check + 1;
                function.chunk.code[branch] = Instruction::Jump {
                    offset: JumpOffset::new(-(branch as i32)),
                };
            }
            _ => unreachable!(),
        }
        let units = [&unit.unit];
        let world = World::new(&units, &[]);
        let (optimized, _) = optimize_function(
            &unit.unit,
            0,
            Vec::new(),
            &world,
            &unit._heap,
            OptimizationConfiguration {
                copy_propagation: false,
                dead_store: false,
                reuse_temporaries: false,
                ..OptimizationConfiguration::default()
            },
        );
        let Instruction::FillDefault { offset, .. } = optimized.code[0] else {
            panic!("{case}: {:?}", optimized.code);
        };
        let check = relative_target(0, offset.offset());
        assert!(
            matches!(optimized.code[check], Instruction::AsCheck { .. }),
            "{case}: {:?}",
            optimized.code,
        );
    }
}
