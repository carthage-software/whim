//! Renders chunks as deterministic text for debugging and tests.

use std::fmt::Write as _;

use crate::chunk::Chunk;
use crate::chunk::descriptors::CallDescriptor;
use crate::chunk::descriptors::FloatPairUpdateDescriptor;
use crate::chunk::descriptors::FloatSquaresSumBranchDescriptor;
use crate::chunk::descriptors::IcDescriptor;
use crate::chunk::descriptors::IntStepLoopDescriptor;
use crate::chunk::descriptors::Literal;
use crate::chunk::descriptors::PreparedIntLoopDescriptor;
use crate::chunk::descriptors::PresetDescriptor;
use crate::chunk::descriptors::PresetSlot;
use crate::chunk::descriptors::PropertyInitializationDescriptor;
use crate::chunk::descriptors::SwitchTable;
use crate::chunk::descriptors::TypeDescriptor;
use crate::disassemble::operands::operands;
use crate::disassemble::render::call_descriptor;
use crate::disassemble::render::descriptor_reference;
use crate::disassemble::render::float_pair_update_descriptor;
use crate::disassemble::render::float_squares_sum_branch_descriptor;
use crate::disassemble::render::ic_descriptor;
use crate::disassemble::render::int_step_loop_descriptor;
use crate::disassemble::render::literal;
use crate::disassemble::render::prepared_int_loop_descriptor;
use crate::disassemble::render::preset_shape;
use crate::disassemble::render::property_initialization_descriptor;
use crate::disassemble::render::switch_table;
use crate::disassemble::render::type_descriptor;
use crate::instruction::Instruction;
use crate::instruction::operands::CallDescriptorIndex;
use crate::instruction::operands::ConstantIndex;
use crate::instruction::operands::DescriptorIndex;
use crate::instruction::operands::IcSlot;
use crate::instruction::operands::IndexAddMode;
use crate::instruction::operands::JumpOffset;
use crate::instruction::operands::PresetDescriptorIndex;
use crate::instruction::operands::Register;
use crate::instruction::operands::ShortJumpOffset;
use crate::instruction::operands::SwitchTableIndex;

mod operands;
mod render;

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
pub fn disassemble(chunk: &Chunk, name: &str) -> String {
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
