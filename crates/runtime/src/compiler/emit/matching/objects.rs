use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::cst::control_flow::Match;
use whim_syn::cst::pattern::ObjectPattern;
use whim_syn::cst::pattern::ObjectPatternEntry;
use whim_syn::cst::pattern::Pattern;
use whim_syn::cst::pattern::TrailingPattern;
use whim_syn::cst::sequence::TokenSeparatedSequence;
use whim_syn::cst::r#type::Type;

use super::BodyCompiler;
use super::CompileError;
use super::IcDescriptor;
use super::ImmediateInt;
use super::Instruction;
use super::JumpOffset;
use super::Register;
use super::Scope;
use super::TypeDescriptor;
use super::lower_pattern_type;
use super::pattern_has_bindings;
use super::tuple_index;

pub(super) fn contains_object_pattern(pattern: &Pattern<'_>) -> bool {
    let nested = contains_object_pattern;
    match pattern {
        Pattern::Object(_) | Pattern::NamedObject(_) => true,
        Pattern::Parenthesized(pattern) => nested(pattern.pattern),
        Pattern::As(pattern) => nested(pattern.left) || nested(pattern.right),
        Pattern::Intersection(pattern) => nested(pattern.left) || nested(pattern.right),
        Pattern::Union(pattern) => nested(pattern.left) || nested(pattern.right),
        Pattern::Vec(pattern) => {
            pattern.elements.iter().any(nested)
                || pattern
                    .trailing
                    .and_then(|rest| rest.pattern)
                    .is_some_and(nested)
        }
        Pattern::Tuple(pattern) => {
            pattern.elements.iter().any(nested)
                || pattern
                    .trailing
                    .and_then(|rest| rest.pattern)
                    .is_some_and(nested)
        }
        Pattern::Dict(pattern) => {
            pattern.entries.iter().any(|entry| nested(entry.pattern))
                || pattern
                    .trailing
                    .and_then(|rest| rest.pattern)
                    .is_some_and(nested)
        }
        Pattern::Wildcard(_) | Pattern::Variable(_) | Pattern::Type(_) => false,
    }
}

fn layout_descriptor(descriptor: &mut TypeDescriptor) {
    match descriptor {
        TypeDescriptor::ObjectShape { entries, .. } => {
            for (_, value) in entries {
                *value = TypeDescriptor::Wildcard;
            }
        }
        TypeDescriptor::DictionaryShape { entries, .. } => {
            for (_, value) in entries {
                *value = TypeDescriptor::Wildcard;
            }
        }
        TypeDescriptor::VectorShape { elements, .. }
        | TypeDescriptor::TupleRest { elements, .. }
        | TypeDescriptor::Tuple(elements) => elements.fill(TypeDescriptor::Wildcard),
        TypeDescriptor::Union(members) => members.iter_mut().for_each(layout_descriptor),
        _ => {}
    }
}

impl BodyCompiler<'_, '_> {
    pub(super) fn object_matching(
        &mut self,
        scope: &Scope<'_>,
        matching: &Match<'_>,
    ) -> Result<Register, CompileError> {
        let result = self.allocate(matching.span())?;
        let subject = self.allocate(matching.span())?;
        let evaluated = self.expression(scope, matching.expression)?;
        self.move_into(subject, evaluated, matching.expression.span());
        let mut exits = Vec::new();
        for arm in &matching.arms {
            let saved = self.save_defined();
            let mark = self.registers.mark();
            let local_count = self.push_pattern_bindings(arm.pattern)?;
            let mut failures = Vec::new();
            self.test_and_bind_pattern(scope, arm.pattern, subject, &mut failures)?;
            let value = self.expression(scope, arm.expression)?;
            self.move_into(result, value, arm.expression.span());
            exits.push(self.chunk.emit(
                Instruction::Jump {
                    offset: JumpOffset::new(0),
                },
                arm.expression.span(),
            ));
            self.truncate_locals(local_count);
            self.restore_defined(saved);
            self.registers.release_to(mark);
            for failure in failures {
                self.chunk.patch_jump(failure, self.code_position());
            }
        }
        self.chunk.emit(
            Instruction::ThrowUnhandledMatch { subject },
            matching.span(),
        );
        for exit in exits {
            self.chunk.patch_jump(exit, self.code_position());
        }
        Ok(result)
    }

    fn test_pattern_descriptor(
        &mut self,
        descriptor: TypeDescriptor,
        value: Register,
        span: Span,
        failures: &mut Vec<u32>,
    ) -> Result<(), CompileError> {
        if matches!(descriptor, TypeDescriptor::Wildcard | TypeDescriptor::Mixed) {
            return Ok(());
        }
        let descriptor = self.add_type_descriptor(descriptor, span)?;
        let comparison = self.allocate(span)?;
        self.chunk.emit(
            Instruction::Is {
                destination: comparison,
                source: value,
                descriptor,
            },
            span,
        );
        failures.push(self.chunk.emit(
            Instruction::JumpIfFalse {
                condition: comparison,
                offset: JumpOffset::new(0),
            },
            span,
        ));
        Ok(())
    }

    fn test_and_bind_pattern(
        &mut self,
        scope: &Scope<'_>,
        pattern: &Pattern<'_>,
        value: Register,
        failures: &mut Vec<u32>,
    ) -> Result<(), CompileError> {
        match pattern {
            Pattern::Wildcard(_) => Ok(()),
            Pattern::Variable(variable) => self.bind_pattern_variable(variable, value),
            Pattern::Parenthesized(pattern) => {
                self.test_and_bind_pattern(scope, pattern.pattern, value, failures)
            }
            Pattern::As(pattern) => {
                self.test_and_bind_pattern(scope, pattern.left, value, failures)?;
                self.test_and_bind_pattern(scope, pattern.right, value, failures)
            }
            Pattern::Intersection(pattern) => {
                self.test_and_bind_pattern(scope, pattern.left, value, failures)?;
                self.test_and_bind_pattern(scope, pattern.right, value, failures)
            }
            Pattern::Union(pattern) => {
                let mut left_failures = Vec::new();
                self.test_and_bind_pattern(scope, pattern.left, value, &mut left_failures)?;
                let exit = self.chunk.emit(
                    Instruction::Jump {
                        offset: JumpOffset::new(0),
                    },
                    pattern.span(),
                );
                for failure in left_failures {
                    self.chunk.patch_jump(failure, self.code_position());
                }
                self.test_and_bind_pattern(scope, pattern.right, value, failures)?;
                self.chunk.patch_jump(exit, self.code_position());
                Ok(())
            }
            Pattern::Object(object) => self.test_and_bind_object(scope, object, value, failures),
            Pattern::NamedObject(pattern) => {
                let descriptor =
                    lower_pattern_type(&self.types(scope), &Type::Named(pattern.name))?;
                self.test_pattern_descriptor(descriptor, value, pattern.name.span(), failures)?;
                self.test_and_bind_object(scope, &pattern.object, value, failures)
            }
            Pattern::Vec(sequence) if contains_object_pattern(pattern) => self
                .test_and_bind_sequence(
                    scope,
                    pattern,
                    value,
                    failures,
                    (&sequence.elements, sequence.trailing.as_ref()),
                ),
            Pattern::Tuple(sequence) if contains_object_pattern(pattern) => self
                .test_and_bind_sequence(
                    scope,
                    pattern,
                    value,
                    failures,
                    (&sequence.elements, sequence.trailing.as_ref()),
                ),
            Pattern::Dict(dictionary) if contains_object_pattern(pattern) => {
                let mut descriptor = self.lower_match_pattern(scope, pattern)?;
                layout_descriptor(&mut descriptor);
                self.test_pattern_descriptor(descriptor, value, pattern.span(), failures)?;
                for entry in &dictionary.entries {
                    let index = self.pattern_dict_key(&entry.key)?;
                    let extracted = self.allocate(entry.span())?;
                    self.chunk.emit(
                        Instruction::IndexGet {
                            destination: extracted,
                            container: value,
                            index,
                        },
                        entry.span(),
                    );
                    self.test_and_bind_pattern(scope, entry.pattern, extracted, failures)?;
                }
                if let Some(rest) = dictionary
                    .trailing
                    .and_then(|rest| rest.pattern)
                    .filter(|rest| pattern_has_bindings(rest))
                {
                    let remainder = self.dict_pattern_remainder(dictionary, value, rest.span())?;
                    self.bind_pattern(scope, rest, remainder)?;
                }
                Ok(())
            }
            _ => {
                let descriptor = self.lower_match_pattern(scope, pattern)?;
                self.test_pattern_descriptor(descriptor, value, pattern.span(), failures)?;
                self.bind_pattern(scope, pattern, value)
            }
        }
    }
    fn test_and_bind_object(
        &mut self,
        scope: &Scope<'_>,
        object: &ObjectPattern<'_>,
        value: Register,
        failures: &mut Vec<u32>,
    ) -> Result<(), CompileError> {
        let mut descriptor = self.lower_object_pattern(scope, object)?;
        layout_descriptor(&mut descriptor);
        self.test_pattern_descriptor(descriptor, value, object.span(), failures)?;
        let mut properties = Vec::with_capacity(object.entries.len());
        for entry in &object.entries {
            let extracted = self.allocate(entry.span())?;
            let cache = self.add_ic_descriptor(
                IcDescriptor::PublicProperty(self.heap.intern(entry.name().as_bytes())),
                entry.span(),
            )?;
            self.chunk.emit(
                Instruction::PropertyGet {
                    destination: extracted,
                    object: value,
                    cache,
                },
                entry.span(),
            );
            properties.push(extracted);
        }
        for (entry, extracted) in object.entries.iter().zip(properties) {
            match entry {
                ObjectPatternEntry::Property { pattern, .. } => {
                    self.test_and_bind_pattern(scope, pattern, extracted, failures)?;
                }
                ObjectPatternEntry::Shorthand(variable) => {
                    self.bind_pattern_variable(variable, extracted)?;
                }
            }
        }
        Ok(())
    }

    fn test_and_bind_sequence(
        &mut self,
        scope: &Scope<'_>,
        pattern: &Pattern<'_>,
        value: Register,
        failures: &mut Vec<u32>,
        sequence: (
            &TokenSeparatedSequence<'_, Pattern<'_>>,
            Option<&TrailingPattern<'_>>,
        ),
    ) -> Result<(), CompileError> {
        let (elements, trailing) = sequence;
        let mut descriptor = self.lower_match_pattern(scope, pattern)?;
        layout_descriptor(&mut descriptor);
        self.test_pattern_descriptor(descriptor, value, pattern.span(), failures)?;
        for (index, element) in elements.iter().enumerate() {
            let extracted = self.allocate(element.span())?;
            self.chunk.emit(
                Instruction::ElementGet {
                    destination: extracted,
                    subject: value,
                    index: ImmediateInt::new(tuple_index(index)),
                },
                element.span(),
            );
            self.test_and_bind_pattern(scope, element, extracted, failures)?;
        }
        if let Some(rest) = trailing
            .and_then(|rest| rest.pattern)
            .filter(|rest| pattern_has_bindings(rest))
        {
            let extracted = self.allocate(rest.span())?;
            self.chunk.emit(
                Instruction::Rest {
                    destination: extracted,
                    subject: value,
                    from: ImmediateInt::new(tuple_index(elements.len())),
                },
                rest.span(),
            );
            self.bind_pattern(scope, rest, extracted)?;
        }
        Ok(())
    }
}
