//! Same-unit validation of sealed class and interface permissions.

use crate::bytecode::unit::ClassLikeKind;
use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::unit::CompiledUnit;
use crate::compiler::error::CompileError;
use crate::compiler::error::CompileErrorKind;
use crate::value::atom::Atom;

/// Rejects declarations in this unit that provably violate a sealed parent
/// or interface. Permission is checked per direct edge: a declaration must
/// be permitted by each sealed type it directly extends or implements, and
/// a sealed ancestor further up is satisfied by the permitted intermediate.
/// The linker checks declarations loaded from another unit.
pub(in crate::compiler) fn validate_sealed_permissions(
    unit: &CompiledUnit,
) -> Result<(), CompileError> {
    for implementor in &unit.classes {
        if implementor.kind != ClassLikeKind::Interface
            && let Some(parent_reference) = &implementor.parent
            && let Some(restricted) =
                disallowing_class(unit, &parent_reference.name, &implementor.name)
        {
            let parent = restricted.name.to_string_lossy();
            let child = implementor.name.to_string_lossy();
            return Err(CompileError::new(
                CompileErrorKind::SealedPermissionViolation,
                format!("{parent} is sealed and does not permit {child} to extend it"),
                parent_reference.span,
            ));
        }

        for direct in &implementor.interfaces {
            if let Some(restricted) = disallowing_interface(unit, &direct.name, &implementor.name) {
                let interface = restricted.name.to_string_lossy();
                let action = match implementor.kind {
                    ClassLikeKind::Interface => "extend",
                    _ => "implement",
                };
                let implementor = implementor.name.to_string_lossy();
                return Err(CompileError::new(
                    CompileErrorKind::SealedPermissionViolation,
                    format!(
                        "{interface} is sealed and does not permit {implementor} to {action} it"
                    ),
                    direct.span,
                ));
            }
        }
    }

    Ok(())
}

fn disallowing_class<'unit>(
    unit: &'unit CompiledUnit,
    name: &Atom,
    child: &Atom,
) -> Option<&'unit CompiledClassLike> {
    let parent = unit
        .classes
        .iter()
        .find(|class| class.kind == ClassLikeKind::Class && class.name == *name)?;

    parent
        .sealed_to
        .as_ref()
        .is_some_and(|permitted| !permitted.contains(child))
        .then_some(parent)
}

fn disallowing_interface<'unit>(
    unit: &'unit CompiledUnit,
    name: &Atom,
    implementor: &Atom,
) -> Option<&'unit CompiledClassLike> {
    let interface = unit
        .classes
        .iter()
        .find(|class| class.kind == ClassLikeKind::Interface && class.name == *name)?;

    interface
        .sealed_to
        .as_ref()
        .is_some_and(|permitted| !permitted.contains(implementor))
        .then_some(interface)
}
