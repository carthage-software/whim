use whim_span::Span;

use crate::artifact::ArtifactError;
use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::SIDE_TABLE_CAPACITY;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::InstructionSideTableMapper;
use crate::bytecode::instruction::operands::CallDescriptorIndex;
use crate::bytecode::instruction::operands::ConstantIndex;
use crate::bytecode::instruction::operands::DescriptorIndex;
use crate::bytecode::instruction::operands::FloatPairUpdateDescriptorIndex;
use crate::bytecode::instruction::operands::FloatSquaresSumBranchDescriptorIndex;
use crate::bytecode::instruction::operands::IcSlot;
use crate::bytecode::instruction::operands::IntStepLoopDescriptorIndex;
use crate::bytecode::instruction::operands::PreparedIntLoopDescriptorIndex;
use crate::bytecode::instruction::operands::PresetDescriptorIndex;
use crate::bytecode::instruction::operands::PropertyInitializationDescriptorIndex;
use crate::bytecode::instruction::operands::Register;
use crate::bytecode::instruction::operands::SwitchTableIndex;
pub(super) fn main(chunks: Vec<Chunk>) -> Result<Chunk, ArtifactError> {
    let mut merged = Chunk::new();
    if chunks.is_empty() {
        merged.emit(Instruction::ReturnNull, Span::zero());
        return Ok(merged);
    }

    for chunk in chunks {
        append_main(&mut merged, chunk)?;
    }

    merged.refresh_runtime_metadata();
    Ok(merged)
}

fn append_main(target: &mut Chunk, mut source: Chunk) -> Result<(), ArtifactError> {
    let first = target.code.is_empty();
    if !first {
        let Some(Instruction::ReturnNull) = target.code.pop() else {
            return Err(ArtifactError::new(
                "an artifact source main chunk has no implicit return",
            ));
        };
        target.spans.pop();

        let boundary = source.spans.first().copied().unwrap_or_else(Span::zero);
        let registers = target.register_count.max(source.register_count);
        for index in 0..registers {
            target.emit(
                Instruction::Clear {
                    target: Register::new(index),
                },
                boundary,
            );
        }
    }

    let offsets = TableOffsets::from_chunk(target);
    offsets.check_capacity(&source)?;
    let code_offset = u32::try_from(target.code.len())
        .map_err(|_| ArtifactError::new("an artifact main chunk exceeds the format limit"))?;

    for instruction in &mut source.code {
        *instruction = offsets.instruction(*instruction)?;
    }
    for entry in &mut source.catch_table {
        entry.start = entry.start.checked_add(code_offset).ok_or_else(|| {
            ArtifactError::new("an artifact catch range exceeds the format limit")
        })?;
        entry.end = entry.end.checked_add(code_offset).ok_or_else(|| {
            ArtifactError::new("an artifact catch range exceeds the format limit")
        })?;
        entry.handler = entry.handler.checked_add(code_offset).ok_or_else(|| {
            ArtifactError::new("an artifact catch target exceeds the format limit")
        })?;
        entry.type_descriptor = offsets.descriptor(entry.type_descriptor)?;
    }
    for descriptor in &mut source.float_squares_sum_branch_descriptors {
        descriptor.constant = offsets.constant(descriptor.constant)?;
    }
    for descriptor in &mut source.float_pair_update_descriptors {
        descriptor.constant = offsets.constant(descriptor.constant)?;
    }

    target.code.extend(source.code);
    target.spans.extend(source.spans);
    target.constants.extend(source.constants);
    target.type_descriptors.extend(source.type_descriptors);
    target.call_descriptors.extend(source.call_descriptors);
    target.switch_tables.extend(source.switch_tables);
    target.preset_descriptors.extend(source.preset_descriptors);
    target.catch_table.extend(source.catch_table);
    target.ic_descriptors.extend(source.ic_descriptors);
    target
        .prepared_int_loop_descriptors
        .extend(source.prepared_int_loop_descriptors);
    target
        .int_step_loop_descriptors
        .extend(source.int_step_loop_descriptors);
    target
        .float_squares_sum_branch_descriptors
        .extend(source.float_squares_sum_branch_descriptors);
    target
        .float_pair_update_descriptors
        .extend(source.float_pair_update_descriptors);
    target
        .property_initialization_descriptors
        .extend(source.property_initialization_descriptors);

    if first {
        target.uninitialized_registers = source.uninitialized_registers;
    }
    target.local_register_count = target.local_register_count.max(source.local_register_count);
    target.register_count = target.register_count.max(source.register_count);
    Ok(())
}

#[derive(Clone, Copy)]
struct TableOffsets {
    constants: usize,
    descriptors: usize,
    calls: usize,
    switches: usize,
    presets: usize,
    caches: usize,
    prepared_int_loops: usize,
    int_step_loops: usize,
    float_square_branches: usize,
    float_pair_updates: usize,
    property_initializations: usize,
}

impl TableOffsets {
    const fn from_chunk(chunk: &Chunk) -> Self {
        Self {
            constants: chunk.constants.len(),
            descriptors: chunk.type_descriptors.len(),
            calls: chunk.call_descriptors.len(),
            switches: chunk.switch_tables.len(),
            presets: chunk.preset_descriptors.len(),
            caches: chunk.ic_descriptors.len(),
            prepared_int_loops: chunk.prepared_int_loop_descriptors.len(),
            int_step_loops: chunk.int_step_loop_descriptors.len(),
            float_square_branches: chunk.float_squares_sum_branch_descriptors.len(),
            float_pair_updates: chunk.float_pair_update_descriptors.len(),
            property_initializations: chunk.property_initialization_descriptors.len(),
        }
    }

    fn check_capacity(self, source: &Chunk) -> Result<(), ArtifactError> {
        check_table(self.constants, source.constants.len(), "constants")?;
        check_table(
            self.descriptors,
            source.type_descriptors.len(),
            "type descriptors",
        )?;
        check_table(
            self.calls,
            source.call_descriptors.len(),
            "call descriptors",
        )?;
        check_table(self.switches, source.switch_tables.len(), "switch tables")?;
        check_table(
            self.presets,
            source.preset_descriptors.len(),
            "preset descriptors",
        )?;
        check_table(self.caches, source.ic_descriptors.len(), "inline caches")?;
        check_table(
            self.prepared_int_loops,
            source.prepared_int_loop_descriptors.len(),
            "prepared integer-loop descriptors",
        )?;
        check_table(
            self.int_step_loops,
            source.int_step_loop_descriptors.len(),
            "integer-step-loop descriptors",
        )?;
        check_table(
            self.float_square_branches,
            source.float_squares_sum_branch_descriptors.len(),
            "float-square-branch descriptors",
        )?;
        check_table(
            self.float_pair_updates,
            source.float_pair_update_descriptors.len(),
            "float-pair-update descriptors",
        )?;
        check_table(
            self.property_initializations,
            source.property_initialization_descriptors.len(),
            "property-initialization descriptors",
        )
    }

    fn instruction(mut self, mut instruction: Instruction) -> Result<Instruction, ArtifactError> {
        instruction.try_map_side_tables(&mut self)?;
        Ok(instruction)
    }

    fn constant(self, index: ConstantIndex) -> Result<ConstantIndex, ArtifactError> {
        Ok(ConstantIndex::new(rebase(index.index(), self.constants)?))
    }

    fn descriptor(self, index: DescriptorIndex) -> Result<DescriptorIndex, ArtifactError> {
        Ok(DescriptorIndex::new(rebase(
            index.index(),
            self.descriptors,
        )?))
    }

    fn call(self, index: CallDescriptorIndex) -> Result<CallDescriptorIndex, ArtifactError> {
        Ok(CallDescriptorIndex::new(rebase(index.index(), self.calls)?))
    }

    fn switch(self, index: SwitchTableIndex) -> Result<SwitchTableIndex, ArtifactError> {
        Ok(SwitchTableIndex::new(rebase(index.index(), self.switches)?))
    }

    fn preset(self, index: PresetDescriptorIndex) -> Result<PresetDescriptorIndex, ArtifactError> {
        Ok(PresetDescriptorIndex::new(rebase(
            index.index(),
            self.presets,
        )?))
    }

    fn cache(self, index: IcSlot) -> Result<IcSlot, ArtifactError> {
        Ok(IcSlot::new(rebase(index.index(), self.caches)?))
    }

    fn prepared_int_loop(
        self,
        index: PreparedIntLoopDescriptorIndex,
    ) -> Result<PreparedIntLoopDescriptorIndex, ArtifactError> {
        Ok(PreparedIntLoopDescriptorIndex::new(rebase(
            index.index(),
            self.prepared_int_loops,
        )?))
    }

    fn int_step_loop(
        self,
        index: IntStepLoopDescriptorIndex,
    ) -> Result<IntStepLoopDescriptorIndex, ArtifactError> {
        Ok(IntStepLoopDescriptorIndex::new(rebase(
            index.index(),
            self.int_step_loops,
        )?))
    }

    fn float_square_branch(
        self,
        index: FloatSquaresSumBranchDescriptorIndex,
    ) -> Result<FloatSquaresSumBranchDescriptorIndex, ArtifactError> {
        Ok(FloatSquaresSumBranchDescriptorIndex::new(rebase(
            index.index(),
            self.float_square_branches,
        )?))
    }

    fn float_pair_update(
        self,
        index: FloatPairUpdateDescriptorIndex,
    ) -> Result<FloatPairUpdateDescriptorIndex, ArtifactError> {
        Ok(FloatPairUpdateDescriptorIndex::new(rebase(
            index.index(),
            self.float_pair_updates,
        )?))
    }

    fn property_initialization(
        self,
        index: PropertyInitializationDescriptorIndex,
    ) -> Result<PropertyInitializationDescriptorIndex, ArtifactError> {
        Ok(PropertyInitializationDescriptorIndex::new(rebase(
            index.index(),
            self.property_initializations,
        )?))
    }
}

impl InstructionSideTableMapper for TableOffsets {
    type Error = ArtifactError;

    fn constant(&mut self, value: ConstantIndex) -> Result<ConstantIndex, Self::Error> {
        Self::constant(*self, value)
    }

    fn cache(&mut self, value: IcSlot) -> Result<IcSlot, Self::Error> {
        Self::cache(*self, value)
    }

    fn switch(&mut self, value: SwitchTableIndex) -> Result<SwitchTableIndex, Self::Error> {
        Self::switch(*self, value)
    }

    fn descriptor(&mut self, value: DescriptorIndex) -> Result<DescriptorIndex, Self::Error> {
        Self::descriptor(*self, value)
    }

    fn call(&mut self, value: CallDescriptorIndex) -> Result<CallDescriptorIndex, Self::Error> {
        Self::call(*self, value)
    }

    fn preset(
        &mut self,
        value: PresetDescriptorIndex,
    ) -> Result<PresetDescriptorIndex, Self::Error> {
        Self::preset(*self, value)
    }

    fn float_pair_update(
        &mut self,
        value: FloatPairUpdateDescriptorIndex,
    ) -> Result<FloatPairUpdateDescriptorIndex, Self::Error> {
        Self::float_pair_update(*self, value)
    }

    fn float_squares_sum_branch(
        &mut self,
        value: FloatSquaresSumBranchDescriptorIndex,
    ) -> Result<FloatSquaresSumBranchDescriptorIndex, Self::Error> {
        Self::float_square_branch(*self, value)
    }

    fn int_step_loop(
        &mut self,
        value: IntStepLoopDescriptorIndex,
    ) -> Result<IntStepLoopDescriptorIndex, Self::Error> {
        Self::int_step_loop(*self, value)
    }

    fn prepared_int_loop(
        &mut self,
        value: PreparedIntLoopDescriptorIndex,
    ) -> Result<PreparedIntLoopDescriptorIndex, Self::Error> {
        Self::prepared_int_loop(*self, value)
    }

    fn property_initialization(
        &mut self,
        value: PropertyInitializationDescriptorIndex,
    ) -> Result<PropertyInitializationDescriptorIndex, Self::Error> {
        Self::property_initialization(*self, value)
    }
}

fn check_table(current: usize, added: usize, name: &str) -> Result<(), ArtifactError> {
    if current
        .checked_add(added)
        .is_none_or(|length| length > SIDE_TABLE_CAPACITY)
    {
        return Err(ArtifactError::new(format!(
            "an artifact main chunk has too many {name}",
        )));
    }
    Ok(())
}

fn rebase(index: u16, offset: usize) -> Result<u16, ArtifactError> {
    usize::from(index)
        .checked_add(offset)
        .and_then(|index| u16::try_from(index).ok())
        .ok_or_else(|| ArtifactError::new("an artifact side-table index exceeds the format limit"))
}
