use whim_syn::cst::array::ArrayAccess;
use whim_syn::cst::operation::Binary;

use crate::compiler::emit::Access;
use crate::compiler::emit::BodyCompiler;
use crate::compiler::emit::CompileError;
use crate::compiler::emit::Expression;
use crate::compiler::emit::HasSpan;
use crate::compiler::emit::IcDescriptor;
use crate::compiler::emit::Instruction;
use crate::compiler::emit::JumpOffset;
use crate::compiler::emit::Register;
use crate::compiler::emit::Scope;

impl BodyCompiler<'_, '_> {
    pub(in crate::compiler::emit) fn coalesce(
        &mut self,
        scope: &Scope<'_>,
        binary: &Binary<'_>,
    ) -> Result<Register, CompileError> {
        let destination = self.allocate(binary.span())?;
        let saved = self.save_defined();
        let escapes = self.coalescing_read(scope, binary.lhs, destination)?;
        let skip = self.chunk.emit(
            Instruction::JumpIfNotNull {
                subject: destination,
                offset: JumpOffset::new(0),
            },
            binary.operator.span(),
        );
        let fallback = self.code_position();
        for escape in escapes {
            self.chunk.patch_jump(escape, fallback);
        }
        self.restore_defined(saved.clone());
        let right = self.expression(scope, binary.rhs)?;
        self.move_into(destination, right, binary.rhs.span());
        self.restore_defined(saved);
        self.chunk.patch_jump(skip, self.code_position());
        Ok(destination)
    }

    fn coalescing_read(
        &mut self,
        scope: &Scope<'_>,
        expression: &Expression<'_>,
        destination: Register,
    ) -> Result<Vec<u32>, CompileError> {
        let mut links = Vec::new();
        let mut current = expression;
        let foot = loop {
            match current {
                Expression::Parenthesized(parenthesized) => current = parenthesized.expression,
                Expression::ArrayAccess(access) => {
                    links.push(current);
                    current = access.array;
                }
                Expression::Access(Access::Property(access)) => {
                    links.push(current);
                    current = access.object;
                }
                Expression::Access(Access::NullSafeProperty(access)) => {
                    links.push(current);
                    current = access.object;
                }
                _ => break current,
            }
        };
        let mut receiver = match foot {
            Expression::Access(Access::StaticProperty(access)) => {
                let cache = self.static_property_cache(scope, access)?;
                let target = if links.is_empty() {
                    destination
                } else {
                    self.allocate(foot.span())?
                };
                self.chunk.emit(
                    Instruction::StaticPropertyGetOrNull {
                        destination: target,
                        cache,
                    },
                    foot.span(),
                );
                target
            }
            _ => self.expression(scope, foot)?,
        };
        let mut escapes = Vec::with_capacity(links.len());
        let count = links.len();
        for (index, link) in links.into_iter().rev().enumerate() {
            escapes.push(self.chunk.emit(
                Instruction::JumpIfNull {
                    subject: receiver,
                    offset: JumpOffset::new(0),
                },
                link.span(),
            ));
            let target = if index + 1 == count {
                destination
            } else {
                self.allocate(link.span())?
            };
            let instruction = match link {
                Expression::ArrayAccess(access) => {
                    self.coalescing_index(scope, access, receiver, target)?
                }
                Expression::Access(access) => {
                    let property = match access {
                        Access::Property(access) => &access.property,
                        Access::NullSafeProperty(access) => &access.property,
                        _ => unreachable!(),
                    };
                    let cache = self.add_ic_descriptor(
                        IcDescriptor::Member {
                            name: self.heap.intern(property.value.as_bytes()),
                            type_arguments: None,
                        },
                        link.span(),
                    )?;
                    Instruction::PropertyGetOrNull {
                        destination: target,
                        object: receiver,
                        cache,
                    }
                }
                _ => unreachable!(),
            };
            self.chunk.emit(instruction, link.span());
            receiver = target;
        }
        if receiver != destination {
            self.move_into(destination, receiver, expression.span());
        }
        Ok(escapes)
    }

    fn coalescing_index(
        &mut self,
        scope: &Scope<'_>,
        access: &ArrayAccess<'_>,
        mut container: Register,
        destination: Register,
    ) -> Result<Instruction, CompileError> {
        if container.index() < self.registers.temporary_floor()
            && !matches!(
                access.index.unparenthesized(),
                Expression::Literal(_) | Expression::Variable(_)
            )
        {
            let snapshot = self.allocate(access.array.span())?;
            self.move_into(snapshot, container, access.array.span());
            container = snapshot;
        }
        Ok(Instruction::IndexGetOrNull {
            destination,
            container,
            index: self.expression(scope, access.index)?,
        })
    }
}
