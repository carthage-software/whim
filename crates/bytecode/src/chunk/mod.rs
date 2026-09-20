//! One compiled code object: instructions plus the side tables they
//! reference.

use hashbrown::HashMap;
use serde::Deserialize;
use serde::Serialize;
use serde_seeded::DeserializeSeeded;
use whim_base::u32_index;
use whim_span::Span;
use whim_value::heap::Heap;

use crate::REFERENCE_REGISTER_LIMIT;
use crate::instruction::Instruction;
use crate::instruction::operands::CallDescriptorIndex;
use crate::instruction::operands::Comparison;
use crate::instruction::operands::ConstantIndex;
use crate::instruction::operands::DescriptorIndex;
use crate::instruction::operands::FloatPairUpdateDescriptorIndex;
use crate::instruction::operands::FloatSquaresSumBranchDescriptorIndex;
use crate::instruction::operands::IcSlot;
use crate::instruction::operands::IntStepLoopDescriptorIndex;
use crate::instruction::operands::JumpOffset;
use crate::instruction::operands::PreparedIntLoopDescriptorIndex;
use crate::instruction::operands::PresetDescriptorIndex;
use crate::instruction::operands::PropertyInitializationDescriptorIndex;
use crate::instruction::operands::Register;
use crate::instruction::operands::SwitchTableIndex;
use crate::reference_registers;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SideTable {
    Constants,
    TypeDescriptors,
    CallDescriptors,
    SwitchTables,
    PresetDescriptors,
    InlineCaches,
    NumericDescriptors,
    PropertyInitializers,
}

impl SideTable {
    /// What this table counts, phrased for a diagnostic: "a function may
    /// contain at most 65536 *distinct constants*".
    #[must_use]
    pub const fn counts(self) -> &'static str {
        match self {
            Self::Constants => "distinct constants",
            Self::TypeDescriptors => "type annotations",
            Self::CallDescriptors => "call sites",
            Self::SwitchTables => "match tables",
            Self::PresetDescriptors => "preset descriptors",
            Self::InlineCaches => "property and method access sites",
            Self::NumericDescriptors => "fused numeric operations",
            Self::PropertyInitializers => "fused property initializers",
        }
    }
}

pub mod descriptors;

use crate::chunk::descriptors::CallDescriptor;
use crate::chunk::descriptors::CatchEntry;
use crate::chunk::descriptors::FloatPairUpdateDescriptor;
use crate::chunk::descriptors::FloatSquaresSumBranchDescriptor;
use crate::chunk::descriptors::IcDescriptor;
use crate::chunk::descriptors::IntStepLoopDescriptor;
use crate::chunk::descriptors::Literal;
use crate::chunk::descriptors::PreparedIntLoopDescriptor;
use crate::chunk::descriptors::PresetDescriptor;
use crate::chunk::descriptors::PropertyInitializationDescriptor;
use crate::chunk::descriptors::SwitchTable;
use crate::chunk::descriptors::TypeDescriptor;
use crate::chunk::descriptors::literal_key;

pub const SIDE_TABLE_CAPACITY: usize = u16::MAX as usize + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideTableFull {
    pub table: SideTable,
}

fn next_index(length: usize, table: SideTable) -> Result<u16, SideTableFull> {
    u16::try_from(length).map_err(|_| SideTableFull { table })
}

#[derive(Debug, Clone, Default, Serialize, DeserializeSeeded)]
#[seeded(de(seed(Heap)))]
pub struct Chunk {
    #[seeded(with(serde_seeded::unseeded))]
    pub code: Vec<Instruction>,
    /// The source span of each instruction, parallel to `code`.
    #[seeded(with(serde_seeded::unseeded))]
    pub spans: Vec<Span>,
    pub constants: Vec<Literal>,
    pub type_descriptors: Vec<TypeDescriptor>,
    pub call_descriptors: Vec<CallDescriptor>,
    pub switch_tables: Vec<SwitchTable>,
    /// The partial-application shapes; an empty shape is a first-class
    /// callable, a pure binding.
    pub preset_descriptors: Vec<PresetDescriptor>,
    /// The protected regions, innermost-first.
    #[seeded(with(serde_seeded::unseeded))]
    pub catch_table: Vec<CatchEntry>,
    /// The inline-cache descriptors, one per cache site; their number is the
    /// chunk's cache slot count.
    pub ic_descriptors: Vec<IcDescriptor>,
    #[seeded(with(serde_seeded::unseeded))]
    pub prepared_int_loop_descriptors: Vec<PreparedIntLoopDescriptor>,
    #[seeded(with(serde_seeded::unseeded))]
    pub int_step_loop_descriptors: Vec<IntStepLoopDescriptor>,
    #[seeded(with(serde_seeded::unseeded))]
    pub float_squares_sum_branch_descriptors: Vec<FloatSquaresSumBranchDescriptor>,
    #[seeded(with(serde_seeded::unseeded))]
    pub float_pair_update_descriptors: Vec<FloatPairUpdateDescriptor>,
    #[seeded(with(serde_seeded::unseeded))]
    pub property_initialization_descriptors: Vec<PropertyInitializationDescriptor>,
    /// The leading registers reserved for named locals; they have
    /// source-level lifetime and are never reused for compiler temporaries.
    #[seeded(with(serde_seeded::unseeded))]
    pub local_register_count: u16,
    /// The first parameter register. Parameters are consecutive from here.
    #[seeded(with(serde_seeded::unseeded))]
    pub parameter_register_start: u16,
    /// The number of consecutive parameter registers.
    #[seeded(with(serde_seeded::unseeded))]
    pub parameter_register_count: u16,
    /// Empty when no parameter can be reassigned. Otherwise, one entry per
    /// parameter: `NONE` reads the parameter register itself, while another
    /// register retains the original value.
    #[seeded(with(serde_seeded::unseeded))]
    pub trace_argument_registers: Vec<Register>,
    /// Locals that begin each invocation with the uninitialized sentinel.
    #[seeded(with(serde_seeded::unseeded))]
    pub uninitialized_registers: Vec<Register>,
    #[seeded(with(serde_seeded::unseeded))]
    pub register_count: u16,
    /// Registers represented by the reference ownership mask. Wider frames
    /// use ordinary full-window teardown.
    #[seeded(with(serde_seeded::unseeded))]
    pub reference_register_mask: u64,
    #[seeded(with(serde_seeded::unseeded))]
    pub vec_append_register_mask: u64,
    #[serde(skip)]
    #[seeded(skip)]
    constant_index: Option<HashMap<descriptors::LiteralKey, ConstantIndex>>,
}

impl Chunk {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn clone_tail(&self, start: usize) -> Self {
        debug_assert!(start <= self.code.len());
        debug_assert!(self.catch_table.is_empty());

        Self {
            code: self.code[start..].to_vec(),
            spans: self.spans[start..].to_vec(),
            constants: self.constants.clone(),
            type_descriptors: self.type_descriptors.clone(),
            call_descriptors: self.call_descriptors.clone(),
            switch_tables: self.switch_tables.clone(),
            preset_descriptors: self.preset_descriptors.clone(),
            catch_table: Vec::new(),
            ic_descriptors: self.ic_descriptors.clone(),
            prepared_int_loop_descriptors: self.prepared_int_loop_descriptors.clone(),
            int_step_loop_descriptors: self.int_step_loop_descriptors.clone(),
            float_squares_sum_branch_descriptors: self.float_squares_sum_branch_descriptors.clone(),
            float_pair_update_descriptors: self.float_pair_update_descriptors.clone(),
            property_initialization_descriptors: self.property_initialization_descriptors.clone(),
            local_register_count: self.local_register_count,
            parameter_register_start: self.parameter_register_start,
            parameter_register_count: self.parameter_register_count,
            trace_argument_registers: self.trace_argument_registers.clone(),
            uninitialized_registers: self.uninitialized_registers.clone(),
            register_count: self.register_count,
            reference_register_mask: self.reference_register_mask,
            vec_append_register_mask: self.vec_append_register_mask,
            constant_index: None,
        }
    }

    #[must_use]
    pub fn with_replaced_tail(&self, start: usize, mut tail: Self) -> Self {
        let mut code = Vec::with_capacity(start + tail.code.len());
        code.extend_from_slice(&self.code[..start]);
        code.append(&mut tail.code);
        tail.code = code;

        let mut spans = Vec::with_capacity(start + tail.spans.len());
        spans.extend_from_slice(&self.spans[..start]);
        spans.append(&mut tail.spans);
        tail.spans = spans;
        tail.reference_register_mask |= self.reference_register_mask;
        tail.vec_append_register_mask |= self.vec_append_register_mask;
        tail
    }

    /// Recomputes the conservative set of registers that may own a
    /// reference-counted value.
    pub fn refresh_runtime_metadata(&mut self) {
        self.reference_register_mask = reference_registers::mask(self);
        self.vec_append_register_mask = self.code.iter().fold(0, |mask, instruction| {
            if let Instruction::VecAppend { container, .. } = instruction
                && container.index() < REFERENCE_REGISTER_LIMIT
            {
                mask | (1u64 << container.index())
            } else {
                mask
            }
        });
    }

    /// # Panics
    ///
    /// Panics if the instruction index exceeds [`u32::MAX`].
    pub fn emit(&mut self, instruction: Instruction, span: Span) -> u32 {
        let index = u32_index(self.code.len());
        self.code.push(instruction);
        self.spans.push(span);
        index
    }

    /// Rewrites the jump at `at` to land on instruction `target`, computing
    /// the offset from the jump's own index.
    ///
    /// # Panics
    ///
    /// Panics if `at` does not name a patchable jump or the offset exceeds `i32`.
    pub fn patch_jump(&mut self, at: u32, target: u32) {
        let relative = i32::try_from(i64::from(target) - i64::from(at))
            .expect("a jump offset must fit in i32");
        match &mut self.code[at as usize] {
            Instruction::Jump { offset }
            | Instruction::JumpIfFalse { offset, .. }
            | Instruction::JumpIfTrue { offset, .. }
            | Instruction::JumpIfNull { offset, .. }
            | Instruction::JumpIfNotNull { offset, .. }
            | Instruction::FillDefault { offset, .. } => *offset = JumpOffset::new(relative),
            _ => panic!("only a jump instruction can be patched"),
        }
    }

    /// # Errors
    ///
    /// Returns [`SideTableFull`] if a new constant would exceed the table's capacity.
    ///
    /// # Panics
    ///
    /// Panics if the constant table already exceeds [`SIDE_TABLE_CAPACITY`].
    pub fn add_constant(&mut self, literal: Literal) -> Result<ConstantIndex, SideTableFull> {
        let key = literal_key(&literal);
        let index = self.constant_index.get_or_insert_with(|| {
            self.constants
                .iter()
                .enumerate()
                .map(|(position, literal)| {
                    let index = u16::try_from(position)
                        .expect("a pooled constant must have a sixteen-bit index");
                    (literal_key(literal), ConstantIndex::new(index))
                })
                .collect()
        });
        if let Some(index) = index.get(&key) {
            return Ok(*index);
        }

        self.push_constant(literal)
    }

    /// Appends `literal` without scanning for a duplicate, for callers that
    /// keep their own [`descriptors::LiteralKey`] index over the pool.
    ///
    /// # Errors
    ///
    /// Returns [`SideTableFull`] if the constant table is full.
    pub fn push_constant(&mut self, literal: Literal) -> Result<ConstantIndex, SideTableFull> {
        let index = next_index(self.constants.len(), SideTable::Constants)?;
        let key = self.constant_index.as_ref().map(|_| literal_key(&literal));
        if let Literal::String(atom) = &literal {
            atom.make_immortal();
        }
        self.constants.push(literal);
        let index = ConstantIndex::new(index);
        if let (Some(constants), Some(key)) = (&mut self.constant_index, key) {
            constants.insert(key, index);
        }

        Ok(index)
    }

    /// # Errors
    ///
    /// Returns [`SideTableFull`] if the type descriptor table is full.
    pub fn add_type_descriptor(
        &mut self,
        descriptor: TypeDescriptor,
    ) -> Result<DescriptorIndex, SideTableFull> {
        let index = next_index(self.type_descriptors.len(), SideTable::TypeDescriptors)?;
        self.type_descriptors.push(descriptor);

        Ok(DescriptorIndex::new(index))
    }

    /// # Errors
    ///
    /// Returns [`SideTableFull`] if the call descriptor table is full.
    pub fn add_call_descriptor(
        &mut self,
        descriptor: CallDescriptor,
    ) -> Result<CallDescriptorIndex, SideTableFull> {
        let index = next_index(self.call_descriptors.len(), SideTable::CallDescriptors)?;
        self.call_descriptors.push(descriptor);

        Ok(CallDescriptorIndex::new(index))
    }

    /// # Errors
    ///
    /// Returns [`SideTableFull`] if the prepared integer loop table is full.
    pub fn add_prepared_int_loop_descriptor(
        &mut self,
        descriptor: PreparedIntLoopDescriptor,
    ) -> Result<PreparedIntLoopDescriptorIndex, SideTableFull> {
        let index = next_index(
            self.prepared_int_loop_descriptors.len(),
            SideTable::NumericDescriptors,
        )?;
        self.prepared_int_loop_descriptors.push(descriptor);

        Ok(PreparedIntLoopDescriptorIndex::new(index))
    }

    /// # Errors
    ///
    /// Returns [`SideTableFull`] if the integer step loop table is full.
    pub fn add_int_step_loop_descriptor(
        &mut self,
        descriptor: IntStepLoopDescriptor,
    ) -> Result<IntStepLoopDescriptorIndex, SideTableFull> {
        let index = next_index(
            self.int_step_loop_descriptors.len(),
            SideTable::NumericDescriptors,
        )?;
        self.int_step_loop_descriptors.push(descriptor);

        Ok(IntStepLoopDescriptorIndex::new(index))
    }

    /// # Errors
    ///
    /// Returns [`SideTableFull`] if the float squares sum branch table is full.
    pub fn add_float_squares_sum_branch_descriptor(
        &mut self,
        descriptor: FloatSquaresSumBranchDescriptor,
    ) -> Result<FloatSquaresSumBranchDescriptorIndex, SideTableFull> {
        let index = next_index(
            self.float_squares_sum_branch_descriptors.len(),
            SideTable::NumericDescriptors,
        )?;

        self.float_squares_sum_branch_descriptors.push(descriptor);

        Ok(FloatSquaresSumBranchDescriptorIndex::new(index))
    }

    /// # Errors
    ///
    /// Returns [`SideTableFull`] if the float pair update table is full.
    pub fn add_float_pair_update_descriptor(
        &mut self,
        descriptor: FloatPairUpdateDescriptor,
    ) -> Result<FloatPairUpdateDescriptorIndex, SideTableFull> {
        let index = next_index(
            self.float_pair_update_descriptors.len(),
            SideTable::NumericDescriptors,
        )?;
        self.float_pair_update_descriptors.push(descriptor);

        Ok(FloatPairUpdateDescriptorIndex::new(index))
    }

    /// # Errors
    ///
    /// Returns [`SideTableFull`] if the property initialization table is full.
    pub fn add_property_initialization_descriptor(
        &mut self,
        descriptor: PropertyInitializationDescriptor,
    ) -> Result<PropertyInitializationDescriptorIndex, SideTableFull> {
        let index = next_index(
            self.property_initialization_descriptors.len(),
            SideTable::PropertyInitializers,
        )?;
        self.property_initialization_descriptors.push(descriptor);

        Ok(PropertyInitializationDescriptorIndex::new(index))
    }

    #[must_use]
    pub fn prepared_int_loop_descriptor(
        &self,
        index: PreparedIntLoopDescriptorIndex,
    ) -> &PreparedIntLoopDescriptor {
        &self.prepared_int_loop_descriptors[usize::from(index.index())]
    }

    #[must_use]
    pub fn int_step_loop_descriptor(
        &self,
        index: IntStepLoopDescriptorIndex,
    ) -> &IntStepLoopDescriptor {
        &self.int_step_loop_descriptors[usize::from(index.index())]
    }

    #[must_use]
    pub fn float_squares_sum_branch_descriptor(
        &self,
        index: FloatSquaresSumBranchDescriptorIndex,
    ) -> &FloatSquaresSumBranchDescriptor {
        &self.float_squares_sum_branch_descriptors[usize::from(index.index())]
    }

    #[must_use]
    pub fn float_pair_update_descriptor(
        &self,
        index: FloatPairUpdateDescriptorIndex,
    ) -> &FloatPairUpdateDescriptor {
        &self.float_pair_update_descriptors[usize::from(index.index())]
    }

    #[must_use]
    pub fn property_initialization_descriptor(
        &self,
        index: PropertyInitializationDescriptorIndex,
    ) -> &PropertyInitializationDescriptor {
        &self.property_initialization_descriptors[usize::from(index.index())]
    }

    /// # Errors
    ///
    /// Returns [`SideTableFull`] if the switch table pool is full.
    pub fn add_switch_table(
        &mut self,
        table: SwitchTable,
    ) -> Result<SwitchTableIndex, SideTableFull> {
        let index = next_index(self.switch_tables.len(), SideTable::SwitchTables)?;
        self.switch_tables.push(table);

        Ok(SwitchTableIndex::new(index))
    }

    /// # Errors
    ///
    /// Returns [`SideTableFull`] if the preset descriptor table is full.
    pub fn add_preset_descriptor(
        &mut self,
        descriptor: PresetDescriptor,
    ) -> Result<PresetDescriptorIndex, SideTableFull> {
        let index = next_index(self.preset_descriptors.len(), SideTable::PresetDescriptors)?;
        self.preset_descriptors.push(descriptor);

        Ok(PresetDescriptorIndex::new(index))
    }

    /// Appends an inline-cache descriptor, allocating the site's slot, and
    /// returns it.
    ///
    /// # Errors
    ///
    /// Returns [`SideTableFull`] if the inline cache descriptor table is full.
    pub fn add_ic_descriptor(&mut self, descriptor: IcDescriptor) -> Result<IcSlot, SideTableFull> {
        let index = next_index(self.ic_descriptors.len(), SideTable::InlineCaches)?;
        self.ic_descriptors.push(descriptor);

        Ok(IcSlot::new(index))
    }
}
