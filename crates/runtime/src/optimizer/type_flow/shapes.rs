use hashbrown::HashMap;

use crate::bytecode::chunk::descriptors::DictionaryTypeDescriptor;
use crate::bytecode::chunk::descriptors::ShapeKey;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::instruction::operands::Register;
use crate::optimizer::type_flow::Fact;
use crate::optimizer::type_flow::TypeFlow;
use crate::optimizer::type_flow::constants::ConstantDictionaryKey;
use crate::optimizer::type_flow::instruction_index;

impl TypeFlow<'_> {
    pub(super) fn sequence_shape_proves(
        &self,
        fact: Fact,
        elements: &[TypeDescriptor],
        rest: Option<&TypeDescriptor>,
        depth: usize,
    ) -> bool {
        let Some(index) = instruction_index(fact.origin) else {
            return false;
        };
        let (element_count, first_element) = match self.chunk.code[index] {
            Instruction::NewVec {
                element_count,
                first_element,
                ..
            }
            | Instruction::NewTuple {
                element_count,
                first_element,
                ..
            } => (usize::from(element_count.value()), first_element),
            _ => return false,
        };
        if element_count < elements.len() || rest.is_none() && element_count != elements.len() {
            return false;
        }

        (0..element_count).all(|position| {
            let register = usize::from(first_element.index()) + position;
            let Some(expected) = elements.get(position).or(rest) else {
                return false;
            };
            register < usize::from(self.chunk.register_count)
                && self.fact_proves(
                    self.fact(index, Register::new(register as u16)),
                    expected,
                    depth + 1,
                )
        })
    }

    pub(super) fn dictionary_shape_proves(
        &self,
        fact: Fact,
        entries: &[(ShapeKey, TypeDescriptor)],
        rest: Option<&DictionaryTypeDescriptor>,
        depth: usize,
    ) -> bool {
        let Some(index) = instruction_index(fact.origin) else {
            return false;
        };
        let Instruction::NewDict {
            pair_count,
            first_pair,
            ..
        } = self.chunk.code[index]
        else {
            return false;
        };

        let mut values = HashMap::with_capacity(usize::from(pair_count.value()));
        for pair in 0..usize::from(pair_count.value()) {
            let key_register = usize::from(first_pair.index()) + pair * 2;
            let value_register = key_register + 1;
            if value_register >= usize::from(self.chunk.register_count) {
                return false;
            }
            let Some(key) = self
                .constant_value_fact(
                    self.fact(index, Register::new(key_register as u16)),
                    depth + 1,
                )
                .and_then(ConstantDictionaryKey::from_value)
            else {
                return false;
            };
            values.insert(key, self.fact(index, Register::new(value_register as u16)));
        }

        if values.len() < entries.len() || rest.is_none() && values.len() != entries.len() {
            return false;
        }
        for (key, expected) in entries {
            let key = match key {
                ShapeKey::Int(value) => ConstantDictionaryKey::Int(*value),
                ShapeKey::String(value) => ConstantDictionaryKey::String(value.clone()),
            };
            let Some(actual) = values.remove(&key) else {
                return false;
            };
            if !self.fact_proves(actual, expected, depth + 1) {
                return false;
            }
        }

        values.into_iter().all(|(key, actual)| {
            let Some((expected_key, expected_value)) = rest else {
                return false;
            };
            let key = match key {
                ConstantDictionaryKey::Bool(true) => TypeDescriptor::TrueLiteral,
                ConstantDictionaryKey::Bool(false) => TypeDescriptor::FalseLiteral,
                ConstantDictionaryKey::Int(value) => TypeDescriptor::IntLiteral(value),
                ConstantDictionaryKey::String(value) => TypeDescriptor::StringLiteral(value),
            };
            self.descriptor_proves(&key, expected_key, depth + 1)
                && self.fact_proves(actual, expected_value, depth + 1)
        })
    }
}
