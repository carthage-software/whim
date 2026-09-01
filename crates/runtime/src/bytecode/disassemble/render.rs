use crate::bytecode::disassemble::CallDescriptor;
use crate::bytecode::disassemble::CallDescriptorIndex;
use crate::bytecode::disassemble::Chunk;
use crate::bytecode::disassemble::ConstantIndex;
use crate::bytecode::disassemble::DescriptorIndex;
use crate::bytecode::disassemble::FloatPairUpdateDescriptor;
use crate::bytecode::disassemble::FloatSquaresSumBranchDescriptor;
use crate::bytecode::disassemble::IcDescriptor;
use crate::bytecode::disassemble::IcSlot;
use crate::bytecode::disassemble::IntStepLoopDescriptor;
use crate::bytecode::disassemble::JumpOffset;
use crate::bytecode::disassemble::Literal;
use crate::bytecode::disassemble::PreparedIntLoopDescriptor;
use crate::bytecode::disassemble::PresetDescriptor;
use crate::bytecode::disassemble::PresetDescriptorIndex;
use crate::bytecode::disassemble::PresetSlot;
use crate::bytecode::disassemble::PropertyInitializationDescriptor;
use crate::bytecode::disassemble::Register;
use crate::bytecode::disassemble::ShortJumpOffset;
use crate::bytecode::disassemble::SwitchTable;
use crate::bytecode::disassemble::SwitchTableIndex;
use crate::bytecode::disassemble::TypeDescriptor;
use crate::bytecode::render;

pub(crate) fn register(register: Register) -> String {
    if register == Register::NONE {
        "none".to_string()
    } else {
        format!("r{}", register.index())
    }
}

pub(crate) fn window(first: Register, count: u32) -> String {
    let start = u32::from(first.index());
    format!("r{start}..r{}", start + count)
}

/// Renders a short jump's relative offset as an absolute target.
pub(crate) fn short_jump(index: usize, offset: ShortJumpOffset) -> String {
    jump_target(index, i64::from(offset.offset()))
}

pub(crate) fn jump(index: usize, offset: JumpOffset) -> String {
    jump_target(index, i64::from(offset.offset()))
}

fn jump_target(index: usize, offset: i64) -> String {
    let Ok(index) = i64::try_from(index) else {
        return "-> ????".to_string();
    };
    let target = index.saturating_add(offset);
    format!("-> {target:04}")
}

pub(crate) fn constant_reference(chunk: &Chunk, index: ConstantIndex) -> String {
    let rendered = chunk
        .constants
        .get(usize::from(index.index()))
        .map_or_else(|| "?".to_string(), literal);
    format!("constants[{}]={rendered}", index.index())
}

pub(crate) fn descriptor_reference(chunk: &Chunk, index: DescriptorIndex) -> String {
    let rendered = chunk
        .type_descriptors
        .get(usize::from(index.index()))
        .map_or_else(|| "?".to_string(), type_descriptor);
    format!("descriptors[{}]={rendered}", index.index())
}

pub(crate) fn call_reference(index: CallDescriptorIndex) -> String {
    format!("calls[{}]", index.index())
}

pub(crate) fn preset_reference(chunk: &Chunk, index: PresetDescriptorIndex) -> String {
    let rendered = chunk
        .preset_descriptors
        .get(usize::from(index.index()))
        .map_or_else(|| "?".to_string(), preset_shape);
    format!("presets[{}]={rendered}", index.index())
}

pub(crate) fn preset_shape(descriptor: &PresetDescriptor) -> String {
    let slots = &descriptor.slots;
    if slots.is_empty() && !descriptor.open_remaining {
        return descriptor.type_arguments.as_ref().map_or_else(
            || "()".to_string(),
            |arguments| format!("::<{}> ()", joined(arguments, ", ")),
        );
    }
    let parts: Vec<String> = slots
        .iter()
        .map(|slot| match slot {
            PresetSlot::GivenPositional => "given".to_string(),
            PresetSlot::HolePositional => "?".to_string(),
            PresetSlot::GivenNamed(name) => {
                format!("{name}: given")
            }
            PresetSlot::HoleNamed(name) => {
                format!("{name}: ?")
            }
        })
        .chain(descriptor.open_remaining.then(|| "...".to_string()))
        .collect();
    let shape = format!("({})", parts.join(", "));
    match &descriptor.type_arguments {
        Some(arguments) => format!("::<{}> {shape}", joined(arguments, ", ")),
        None => shape,
    }
}

pub(crate) fn prepared_int_loop_descriptor(descriptor: &PreparedIntLoopDescriptor) -> String {
    format!(
        "PreparedIntLoop {} {}, {} floats=0x{:016x}",
        descriptor.comparison.operator(),
        register(descriptor.counter),
        register(descriptor.limit),
        descriptor.float_registers,
    )
}

pub(crate) fn int_step_loop_descriptor(descriptor: IntStepLoopDescriptor) -> String {
    format!(
        "IntStepLoop {} {}, {}, step {}",
        descriptor.comparison.operator(),
        register(descriptor.counter),
        register(descriptor.limit),
        register(descriptor.step),
    )
}

pub(crate) fn float_squares_sum_branch_descriptor(
    descriptor: &FloatSquaresSumBranchDescriptor,
) -> String {
    format!(
        "FloatSquaresSumBranch sum {}, squares {}, {}, sources {}, {}, {} constant[{}]",
        register(descriptor.sum_destination),
        register(descriptor.first_square_destination),
        register(descriptor.second_square_destination),
        register(descriptor.first_source),
        register(descriptor.second_source),
        descriptor.comparison.operator(),
        descriptor.constant.index()
    )
}

pub(crate) fn float_pair_update_descriptor(descriptor: &FloatPairUpdateDescriptor) -> String {
    format!(
        "FloatPairUpdate {}, {} constant[{}]; {}, {}, {}, {}",
        register(descriptor.first_destination),
        window(descriptor.first_operand, 3),
        descriptor.constant.index(),
        register(descriptor.second_destination),
        register(descriptor.second_operand),
        register(Register::new(descriptor.second_operand.index() + 1)),
        register(descriptor.second_addend),
    )
}

pub(crate) fn property_initialization_descriptor(
    descriptor: &PropertyInitializationDescriptor,
) -> String {
    descriptor
        .entries
        .iter()
        .map(|entry| {
            format!(
                "{} -> slot[{}]{}",
                register(entry.value),
                entry.slot.index(),
                if entry.value_mode.moves() {
                    ", move"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub(crate) fn table_reference(index: SwitchTableIndex) -> String {
    format!("switches[{}]", index.index())
}

/// Renders an inline-cache reference with its descriptor inline.
pub(crate) fn cache_reference(chunk: &Chunk, slot: IcSlot) -> String {
    let rendered = chunk
        .ic_descriptors
        .get(usize::from(slot.index()))
        .map_or_else(|| "?".to_string(), ic_descriptor);
    format!("cache[{}]={rendered}", slot.index())
}

pub(crate) fn literal(value: &Literal) -> String {
    match value {
        Literal::Null => "null".to_string(),
        Literal::Bool(true) => "true".to_string(),
        Literal::Bool(false) => "false".to_string(),
        Literal::Int(value) => value.to_string(),
        Literal::Float(value) => format!("{value:?}"),
        Literal::String(atom) => format!("'{}'", atom.to_string_lossy()),
    }
}

/// Renders a type descriptor in the language's own notation.
pub(crate) fn type_descriptor(descriptor: &TypeDescriptor) -> String {
    render::type_descriptor(descriptor, &|value| format!("{value:?}"))
}

pub(crate) fn joined(members: &[TypeDescriptor], separator: &str) -> String {
    members
        .iter()
        .map(type_descriptor)
        .collect::<Vec<_>>()
        .join(separator)
}

pub(crate) fn call_descriptor(descriptor: &CallDescriptor) -> String {
    let names = descriptor
        .named
        .iter()
        .map(|name| name.to_string_lossy().into_owned())
        .collect::<Vec<String>>()
        .join(", ");
    format!("positional {}, named [{names}]", descriptor.positional)
}

pub(crate) fn ic_descriptor(descriptor: &IcDescriptor) -> String {
    match descriptor {
        IcDescriptor::Member { name, .. } => name.to_string_lossy().into_owned(),
        IcDescriptor::ClassMember { class, member, .. } => {
            format!("{}::{}", class.to_string_lossy(), member.to_string_lossy())
        }
    }
}

pub(crate) fn switch_table(table: &SwitchTable) -> String {
    match table {
        SwitchTable::Int {
            base,
            targets,
            default,
        } => {
            let rendered = targets
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<String>>()
                .join(", ");
            format!("int base {base}, targets [{rendered}], default {default}")
        }
        SwitchTable::String { arms, default, .. } => {
            let rendered = arms
                .iter()
                .map(|(atom, target)| format!("'{}' -> {target}", atom.to_string_lossy()))
                .collect::<Vec<String>>()
                .join(", ");
            format!("string arms [{rendered}], default {default}")
        }
        SwitchTable::StringByte {
            base,
            targets,
            default,
        } => {
            let rendered = targets
                .iter()
                .enumerate()
                .map(|(offset, target)| format!("{} -> {target}", usize::from(*base) + offset))
                .collect::<Vec<String>>()
                .join(", ");
            format!("string byte targets [{rendered}], default {default}")
        }
        SwitchTable::Pattern {
            descriptors,
            targets,
            default,
        } => {
            let rendered = descriptors
                .iter()
                .zip(targets)
                .map(|(descriptor, target)| format!("{} -> {target}", type_descriptor(descriptor)))
                .collect::<Vec<String>>()
                .join(", ");
            format!("pattern arms [{rendered}], default {default}")
        }
        SwitchTable::DictionaryShape {
            patterns,
            targets,
            default,
            ..
        } => {
            let rendered = patterns
                .iter()
                .zip(targets)
                .map(|(pattern, target)| {
                    let pattern = pattern
                        .iter()
                        .map(type_descriptor)
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("[{pattern}] -> {target}")
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("dictionary shape arms [{rendered}], default {default}")
        }
        SwitchTable::Bool { targets, default } => {
            format!(
                "bool false -> {}, true -> {}, default {default}",
                targets[0], targets[1]
            )
        }
        SwitchTable::Float {
            values,
            targets,
            default,
        } => {
            let rendered = values
                .iter()
                .zip(targets)
                .map(|(value, target)| format!("{value} -> {target}"))
                .collect::<Vec<String>>()
                .join(", ");
            format!("float arms [{rendered}], default {default}")
        }
    }
}
