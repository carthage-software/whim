use whim_bytecode::chunk::Chunk;
use whim_bytecode::instruction::Instruction;
use whim_bytecode::instruction::operands::JumpOffset;
use whim_bytecode::rewrite::control_flow_targets;
use whim_span::Span;

#[test]
#[should_panic(expected = "only a jump instruction can be patched")]
fn patching_a_non_jump_panics() {
    let mut chunk = Chunk::new();
    chunk.emit(Instruction::ReturnNull, Span::zero());
    chunk.patch_jump(0, 0);
}

#[test]
#[should_panic(expected = "a jump offset must fit in i32")]
fn patching_an_overflowing_offset_panics() {
    let mut chunk = Chunk::new();
    chunk.emit(
        Instruction::Jump {
            offset: JumpOffset::new(0),
        },
        Span::zero(),
    );
    chunk.patch_jump(0, u32::MAX);
}

#[test]
#[should_panic(expected = "a bytecode branch target must be non-negative")]
fn reading_a_negative_branch_target_panics() {
    let mut chunk = Chunk::new();
    chunk.emit(
        Instruction::Jump {
            offset: JumpOffset::new(-1),
        },
        Span::zero(),
    );
    let _ = control_flow_targets(&chunk);
}
