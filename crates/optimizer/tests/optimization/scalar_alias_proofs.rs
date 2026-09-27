use whim_base::limits::MAX_TYPE_DEPTH;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::verify::verify_unit;
use whim_optimizer::OptimizationConfiguration;
use whim_optimizer::World;

use super::compile;
use super::optimize_function;

const SOURCE: &str = include_str!("../../../../tests/_fixtures/scalar-alias-proofs.whim");

#[test]
fn scalar_alias_proofs_remove_only_proven_call_checks() {
    let unit = compile(SOURCE, OptimizationConfiguration::default());
    verify_unit(&unit).unwrap();
    for (names, unchecked) in [
        (
            [
                "positive_small",
                "positive_wide",
                "positive_large",
                "positive_signed",
                "positive_float",
                "positive_plain",
                "positive_literal",
                "positive_default",
                "positive_dependent",
            ]
            .as_slice(),
            true,
        ),
        (
            [
                "zero",
                "wrong_kind",
                "unknown",
                "merged",
                "invalid_bound",
                "unresolved_bound",
                "unresolved_argument",
                "unresolved_body",
                "unresolved_hidden",
                "unresolved_recursive",
                "unresolved_union",
                "invalid_union",
            ]
            .as_slice(),
            false,
        ),
    ] {
        for name in names {
            let function = unit
                .functions
                .iter()
                .find(|function| function.name.as_bytes() == name.as_bytes())
                .unwrap();
            assert_eq!(
                function.chunk.code.iter().any(|instruction| matches!(
                    instruction,
                    Instruction::CallNamedUnchecked { .. }
                        | Instruction::CallNamedConstantUnchecked { .. }
                )),
                unchecked,
                "{name}: {:?}",
                function.chunk.code
            );
            assert_eq!(
                function.chunk.code.iter().any(|instruction| matches!(
                    instruction,
                    Instruction::CallNamed { .. } | Instruction::CallNamedDirect { .. }
                )),
                !unchecked,
                "{name}: {:?}",
                function.chunk.code
            );
        }
    }
}

#[test]
fn scalar_alias_proofs_decline_invalid_arity_and_deep_bodies() {
    for case in ["missing", "extra", "depth"] {
        let mut unit = compile(
            r"
use Whim\Marker\NeverInline;
type Bounded<T: uint> = uint;
#[NeverInline]
function take(Bounded<uint> $value): uint { return $value; }
function caller(): uint { return take(2u); }
",
            OptimizationConfiguration {
                enabled: false,
                ..OptimizationConfiguration::default()
            },
        );
        if case == "depth" {
            let mut body = TypeDescriptor::Uint;
            for _ in 0..MAX_TYPE_DEPTH + 2 {
                body = TypeDescriptor::Negated(Box::new(body));
            }
            unit.unit.type_aliases[0].descriptor = body;
        } else {
            let TypeDescriptor::Named { arguments, .. } = unit.unit.functions[0].parameters[0]
                .declared_type
                .as_mut()
                .unwrap()
            else {
                panic!("bounded aliases retain their name");
            };
            *arguments =
                (case == "extra").then(|| vec![TypeDescriptor::Uint, TypeDescriptor::Uint]);
        }
        let units = [&unit.unit];
        let world = World::new(&units, &[]);
        let (chunk, _) = optimize_function(
            &unit.unit,
            1,
            Vec::new(),
            &world,
            &unit._heap,
            OptimizationConfiguration::default(),
        );
        assert!(
            chunk.code.iter().any(|instruction| matches!(
                instruction,
                Instruction::CallNamed { .. } | Instruction::CallNamedDirect { .. }
            )),
            "{case}: {:?}",
            chunk.code
        );
        assert!(
            !chunk.code.iter().any(|instruction| matches!(
                instruction,
                Instruction::CallNamedUnchecked { .. }
                    | Instruction::CallNamedConstantUnchecked { .. }
            )),
            "{case}: {:?}",
            chunk.code
        );
    }
}
