//! Renders chunks as deterministic text for debugging and tests.

use std::fmt::Write as _;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::CallDescriptor;
use crate::bytecode::chunk::descriptors::FloatPairUpdateDescriptor;
use crate::bytecode::chunk::descriptors::FloatSquaresSumBranchDescriptor;
use crate::bytecode::chunk::descriptors::IcDescriptor;
use crate::bytecode::chunk::descriptors::IntStepLoopDescriptor;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::PreparedIntLoopDescriptor;
use crate::bytecode::chunk::descriptors::PresetDescriptor;
use crate::bytecode::chunk::descriptors::PresetSlot;
use crate::bytecode::chunk::descriptors::PropertyInitializationDescriptor;
use crate::bytecode::chunk::descriptors::SwitchTable;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::disassemble::operands::operands;
use crate::bytecode::disassemble::render::call_descriptor;
use crate::bytecode::disassemble::render::descriptor_reference;
use crate::bytecode::disassemble::render::float_pair_update_descriptor;
use crate::bytecode::disassemble::render::float_squares_sum_branch_descriptor;
use crate::bytecode::disassemble::render::ic_descriptor;
use crate::bytecode::disassemble::render::int_step_loop_descriptor;
use crate::bytecode::disassemble::render::literal;
use crate::bytecode::disassemble::render::prepared_int_loop_descriptor;
use crate::bytecode::disassemble::render::preset_shape;
use crate::bytecode::disassemble::render::property_initialization_descriptor;
use crate::bytecode::disassemble::render::switch_table;
use crate::bytecode::disassemble::render::type_descriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::CallDescriptorIndex;
use crate::bytecode::instruction::operands::ConstantIndex;
use crate::bytecode::instruction::operands::DescriptorIndex;
use crate::bytecode::instruction::operands::IcSlot;
use crate::bytecode::instruction::operands::IndexAddMode;
use crate::bytecode::instruction::operands::JumpOffset;
use crate::bytecode::instruction::operands::PresetDescriptorIndex;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::instruction::operands::ShortJumpOffset;
use crate::bytecode::instruction::operands::SwitchTableIndex;

mod operands;
pub(crate) mod render;

fn write_section<T>(
    output: &mut String,
    title: &str,
    entries: &[T],
    render: impl Fn(&T) -> String,
) {
    if entries.is_empty() {
        return;
    }

    let _ = writeln!(output, "\n{title}:");
    for (index, entry) in entries.iter().enumerate() {
        let _ = writeln!(output, "  [{index}] {}", render(entry));
    }
}

#[must_use]
pub(crate) fn disassemble(chunk: &Chunk, name: &str) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "== {name} ==");
    let _ = writeln!(output, "registers: {}", chunk.register_count);
    for (index, instruction) in chunk.code.iter().enumerate() {
        let _ = writeln!(
            output,
            "{index:04} {:?}{}",
            instruction.kind(),
            operands(chunk, index, *instruction)
        );
    }

    write_section(&mut output, "constants", &chunk.constants, literal);
    write_section(
        &mut output,
        "type descriptors",
        &chunk.type_descriptors,
        type_descriptor,
    );
    write_section(
        &mut output,
        "call descriptors",
        &chunk.call_descriptors,
        call_descriptor,
    );
    write_section(
        &mut output,
        "cache descriptors",
        &chunk.ic_descriptors,
        ic_descriptor,
    );
    write_section(
        &mut output,
        "preset descriptors",
        &chunk.preset_descriptors,
        preset_shape,
    );
    write_section(
        &mut output,
        "prepared integer loop descriptors",
        &chunk.prepared_int_loop_descriptors,
        prepared_int_loop_descriptor,
    );
    write_section(
        &mut output,
        "integer step-loop descriptors",
        &chunk.int_step_loop_descriptors,
        |descriptor| int_step_loop_descriptor(*descriptor),
    );
    write_section(
        &mut output,
        "float square-sum branch descriptors",
        &chunk.float_squares_sum_branch_descriptors,
        float_squares_sum_branch_descriptor,
    );
    write_section(
        &mut output,
        "float pair-update descriptors",
        &chunk.float_pair_update_descriptors,
        float_pair_update_descriptor,
    );
    write_section(
        &mut output,
        "property initialization descriptors",
        &chunk.property_initialization_descriptors,
        property_initialization_descriptor,
    );
    write_section(
        &mut output,
        "switch tables",
        &chunk.switch_tables,
        switch_table,
    );

    if !chunk.catch_table.is_empty() {
        let _ = writeln!(output, "\ncatch table:");
        for (index, entry) in chunk.catch_table.iter().enumerate() {
            let binding = entry.binding.map_or_else(
                || "none".to_string(),
                |register| format!("r{}", register.index()),
            );

            let _ = writeln!(
                output,
                "  [{index}] {}..{} -> {}, type {}, temporaries r{}.., binding {binding}",
                entry.start,
                entry.end,
                entry.handler,
                descriptor_reference(chunk, entry.type_descriptor),
                entry.temporary_floor,
            );
        }
    }

    output
}
