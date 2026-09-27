use whim_bytecode::chunk::descriptors::IcDescriptor;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::unit::BuiltInCallableAttributes;
use whim_bytecode::unit::CompiledBuiltInFunction;
use whim_bytecode::unit::CompiledUnit;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;
use whim_optimizer::World;

use super::compile;
use super::optimize_function;

fn code<'a>(unit: &'a CompiledUnit, name: &str) -> &'a [Instruction] {
    &unit
        .functions
        .iter()
        .find(|function| function.name.as_bytes() == name.as_bytes())
        .unwrap()
        .chunk
        .code
}

fn has_length(code: &[Instruction]) -> bool {
    code.iter().any(|instruction| {
        matches!(
            instruction,
            Instruction::Length { .. } | Instruction::StringLength { .. }
        )
    })
}

#[test]
fn exact_string_contracts_prove_lengths_and_concatenated_returns() {
    let unit = compile(
        r"
use Whim\Marker\NeverInline;
#[NeverInline]
function byte(string[1] $value): string[1] { return $value; }
#[NeverInline]
function take(string[3] $value): void {}
final class Reader {
    #[NeverInline]
    public function byte(string[1] $value): string[1] { return $value; }
}
function joined(string[2] $left, string[1] $right): string[3] { return $left . $right; }
function left(string[1] $value): string[3] { return 'ab' . $value; }
function right(string[1] $value): string[3] { return $value . 'ab'; }
function bounds(string[1] $value): string[1..=4] { return 'ab' . $value; }
function zero(string[0] $value): string[0] { return $value . $value; }
function copies(string[1] $value): string[3] { $copy = $value; return $copy . $value . $copy; }
function named(string[1] $value): string[3] { return 'ab' . byte($value); }
function method(Reader $reader, string[1] $value): string[3] { return 'ab' . $reader->byte($value); }
function callback(fn(): string[1] $operation): string[3] { return 'ab' . $operation(); }
function width(string[3] $value): uint { return length!($value); }
function named_width(string[1] $value): uint { return length!('ab' . byte($value)); }
function method_width(Reader $reader, string[1] $value): uint { return length!($reader->byte($value)); }
function callback_width(fn(): string[1] $operation): uint { return length!($operation()); }
function pass(string[1] $value): void { take('ab' . $value); }
",
        OptimizationConfiguration::default(),
    );
    verify_unit(&unit).unwrap();
    for name in [
        "joined", "left", "right", "bounds", "zero", "copies", "named", "method", "callback",
    ] {
        let code = code(&unit, name);
        assert!(
            !code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Return { .. })),
            "{name}: {code:?}"
        );
        assert!(
            code.iter().any(|instruction| matches!(
                instruction,
                Instruction::ReturnReferenceUnchecked { .. } | Instruction::ReturnUnchecked { .. }
            )),
            "{name}: {code:?}"
        );
    }
    for name in ["width", "named_width", "method_width", "callback_width"] {
        let code = code(&unit, name);
        assert!(!has_length(code), "{name}: {code:?}");
    }
    assert!(
        !code(&unit, "pass")
            .iter()
            .any(|instruction| matches!(instruction, Instruction::CallNamed { .. }))
    );
}

#[test]
fn unknown_lengths_mutations_and_named_targets_keep_checks() {
    let unit = compile(
        r"
newtype Tagged = string[3];
type Plain = string[3];
type Defaulted<T = Missing> = string[3];
type Bounded<T: Missing|string> = string[3];
type Erase<T> = string[3];
type Hidden = Erase<Missing>;
type Recursive = string|vec<Recursive>;
function plain(string[1] $value): Plain { return 'ab' . $value; }
function tagged(string[1] $value): Tagged { return 'ab' . $value; }
function defaulted(string[1] $value): Defaulted { return 'ab' . $value; }
function bounded(string[1] $value): Bounded<string> { return 'ab' . $value; }
function hidden(string[1] $value): Hidden { return 'ab' . $value; }
function unresolved_union(string[1] $value): Missing|Bounded<string> { return 'ab' . $value; }
function mixed_value(mixed $value): string[3] { return 'ab' . $value; }
function wrong(string[1] $value): string[4] { return 'ab' . $value; }
function changed(string[1] $value, string $other): string[3] { $value = $other; return 'ab' . $value; }
function merged(string[1] $value, string $other, bool $condition): string[3] {
    if ($condition) { $value = $other; }
    return 'ab' . $value;
}
function unknown(string $value): uint { return length!('ab' . $value); }
function range(string[1..=3] $value): uint { return length!($value); }
function recursive(Recursive $value): uint { return length!($value); }
function mutable(#{ value: string[1] } $object, fn(): void $change): string[3] {
    $change();
    return 'ab' . $object->value;
}
",
        OptimizationConfiguration::default(),
    );
    verify_unit(&unit).unwrap();
    for name in [
        "plain",
        "tagged",
        "defaulted",
        "bounded",
        "hidden",
        "unresolved_union",
        "mixed_value",
        "wrong",
        "changed",
        "merged",
        "mutable",
    ] {
        let code = code(&unit, name);
        assert!(
            code.iter()
                .any(|instruction| matches!(instruction, Instruction::Return { .. })),
            "{name}: {code:?}"
        );
    }
    for name in ["unknown", "range", "recursive"] {
        let code = code(&unit, name);
        assert!(has_length(code), "{name}: {code:?}");
    }
}

#[test]
fn native_return_contract_folds_length_and_retains_calls_and_discard_checks() {
    let mut unit = compile(
        r"
function byte(): string[1] { return 'a'; }
function width(): uint { return length!(byte()); }
function discard(): void { byte(); }
",
        OptimizationConfiguration {
            enabled: false,
            ..OptimizationConfiguration::default()
        },
    );
    let declaration = unit.unit.functions.remove(0);
    let built_ins = [CompiledBuiltInFunction {
        name: declaration.name,
        type_parameters: declaration.type_parameters,
        parameters: declaration.parameters,
        return_type: declaration.return_type.unwrap(),
        attributes: BuiltInCallableAttributes {
            must_use: true,
            ..BuiltInCallableAttributes::default()
        },
    }];
    let units = [&unit.unit];
    let world = World::new(&units, &built_ins);
    for (index, discarded) in [(0, false), (1, true)] {
        let (chunk, _) = optimize_function(
            &unit.unit,
            index,
            Vec::new(),
            &world,
            &unit._heap,
            OptimizationConfiguration::default(),
        );
        assert!(!has_length(&chunk.code), "{:?}", chunk.code);
        assert!(chunk.code.iter().any(|instruction| {
            let cache = match instruction {
                Instruction::CallNamed { cache, .. } | Instruction::CallNamedUnchecked { cache, .. }
                | Instruction::CallNamedDiscarded { cache, .. }
                | Instruction::CallNamedConstantUnchecked { cache, .. } => cache,
                _ => return false,
            };
            matches!(&chunk.ic_descriptors[usize::from(cache.index())], IcDescriptor::Member { name, .. } if name.as_bytes() == b"byte")
        }), "{:?}", chunk.code);
        assert_eq!(
            chunk
                .code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::CheckDiscardedResult { .. })),
            discarded,
            "{:?}",
            chunk.code
        );
    }
}

#[test]
fn declared_lengths_reject_negative_ranges_and_checked_sum_overflow() {
    for (min, max) in [
        (-1, Some(-1)),
        (1, Some(2)),
        (0, None),
        (i64::MAX, Some(i64::MAX)),
    ] {
        let mut unit = compile(
            r"
function width(string[1] $value): uint { return length!($value . $value . $value); }
function joined(string[1] $value): string[1..] { return $value . $value; }
",
            OptimizationConfiguration {
                enabled: false,
                ..OptimizationConfiguration::default()
            },
        );
        for function in &mut unit.unit.functions {
            function.parameters[0].declared_type = Some(TypeDescriptor::StringLength { min, max });
        }
        let units = [&unit.unit];
        let world = World::new(&units, &[]);
        let (width, _) = optimize_function(
            &unit.unit,
            0,
            Vec::new(),
            &world,
            &unit._heap,
            OptimizationConfiguration::default(),
        );
        assert!(has_length(&width.code), "{min}, {max:?}: {:?}", width.code);
        let (joined, _) = optimize_function(
            &unit.unit,
            1,
            Vec::new(),
            &world,
            &unit._heap,
            OptimizationConfiguration::default(),
        );
        assert!(
            joined
                .code
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Return { .. })),
            "{min}, {max:?}: {:?}",
            joined.code
        );
    }
}
