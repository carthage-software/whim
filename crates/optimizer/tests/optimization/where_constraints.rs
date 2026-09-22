use super::compile;
use super::method;
use whim_bytecode::instruction::Instruction;
use whim_optimizer::OptimizationConfiguration;

#[test]
fn inlining_keeps_where_checks_in_the_callee_environment() {
    let unit = compile(
        r"
        final class Box<T> {
            #[Whim\Marker\AlwaysInline]
            public function get(): int where T: int { return 1; }

            public function call(): int { return $this->get(); }

            #[Whim\Marker\AlwaysInline]
            public static function choose<U>(): int where U: int { return 2; }
        }
        function choose(): int { return Box::choose::<string>(); }
        $box = new Box::<string>();
        $box->get();
        ",
        OptimizationConfiguration::default(),
    );
    for name in [b"Box::get".as_slice(), b"Box::choose"] {
        assert_eq!(method(&unit, name)[0], Instruction::CheckWhereConstraints);
    }
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
