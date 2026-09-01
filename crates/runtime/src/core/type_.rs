//! Runtime type identities.

use whim_macros::whim_function;
use whim_macros::whim_newtype;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::spec::TypeSpec;
use crate::builtin::throw::Throw;
use crate::symbols::SymbolKind;
use crate::unreachable_invariant;
use crate::value::Value;
use crate::value::newtype::NewtypeId;
use crate::value::object::TypeEnvironmentId;

const TYPE_ID: &[u8] = b"Whim\\Type\\TypeId";

#[whim_newtype("Whim\\Type\\TypeId", "0..")]
pub(crate) struct TypeId;

#[whim_function(
    "Whim\\Type\\of(mixed $value): Whim\\Type\\TypeId",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
pub(crate) fn of(context: &mut Context<'_, '_, '_>, arguments: Arguments<'_>) -> Value {
    // SAFETY: built-in dispatch checked this argument against the declaration.
    let value = unsafe { arguments.value_unchecked(0) };
    let descriptor = context.vm.runtime_type_descriptor(value, 0);
    let identifier = context.vm.intern_type_descriptor(descriptor);
    type_id_value(context, identifier)
}

#[whim_function(
    "Whim\\Type\\id<T>(): Whim\\Type\\TypeId",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
pub(crate) fn id(context: &mut Context<'_, '_, '_>) -> Value {
    let descriptor = context
        .vm
        .built_in_type_descriptor(&TypeSpec::Parameter("T"), context.type_environment);
    let identifier = context.vm.intern_type_descriptor(descriptor);
    type_id_value(context, identifier)
}

#[whim_function(
    "Whim\\Type\\to_debug_string(Whim\\Type\\TypeId $id): string",
    no_track_caller,
    no_trace_boundary,
    must_use
)]
pub(crate) fn to_debug_string(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    // SAFETY: built-in dispatch checked this argument against the declaration.
    let raw = unsafe { arguments.int_unchecked(0) };
    let Ok(raw) = u32::try_from(raw) else {
        return Err(unknown_type_id(context, raw));
    };
    let identifier = TypeEnvironmentId(raw);

    if let Some(rendered) = context
        .vm
        .engine
        .tables
        .type_debug_string_cache
        .get(&identifier)
    {
        return Ok(Value::string(rendered.to_handle()));
    }

    let descriptor = context
        .vm
        .engine
        .tables
        .type_environments
        .get(raw as usize)
        .and_then(|environment| {
            let (name, descriptor) = environment.binding.as_ref()?;
            (environment.parent == Some(TypeEnvironmentId::default())
                && name == &context.vm.engine.tables.type_id_atom)
                .then(|| descriptor.clone())
        })
        .ok_or_else(|| unknown_type_id(context, i64::from(raw)))?;
    let rendered = context.vm.render_descriptor(&descriptor);
    let rendered = context.vm.intern(rendered.as_bytes());
    context
        .vm
        .engine
        .tables
        .type_debug_string_cache
        .insert(identifier, rendered.clone());

    Ok(Value::string(rendered.to_handle()))
}

pub(crate) fn type_id_value(
    context: &mut Context<'_, '_, '_>,
    identifier: TypeEnvironmentId,
) -> Value {
    let name = context.vm.intern(TYPE_ID);
    let Some(entry) = context.vm.engine.tables.symbols.get(&name).copied() else {
        // SAFETY: every engine registers TypeId before it can execute Whim.
        unsafe { unreachable_invariant("the core declares Whim\\Type\\TypeId") }
    };

    if entry.kind != SymbolKind::Newtype {
        // SAFETY: duplicate core symbols are rejected during engine construction.
        unsafe { unreachable_invariant("Whim\\Type\\TypeId is a newtype") }
    }

    let tag = context.vm.engine.tables.intern_newtype_value(
        NewtypeId(entry.index),
        TypeEnvironmentId::default(),
        None,
    );

    Value::newtype(Value::int(i64::from(identifier.0)), tag)
}

fn unknown_type_id(context: &mut Context<'_, '_, '_>, id: i64) -> Throw {
    let class = context.vm.intern(b"Whim\\Unwind\\ValueError");
    context.vm.throw(
        class,
        &format!("the type ID {id} is not known to this engine"),
        0,
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::engine::Engine;
    use crate::engine::EngineConfiguration;
    use crate::symbols::SymbolKind;

    #[test]
    fn type_identity_is_available_without_the_standard_library() {
        let mut engine = Engine::new(EngineConfiguration::default());
        let name = engine.heap.intern(b"Whim\\Type\\TypeId");
        assert_eq!(engine.tables.symbols[&name].kind, SymbolKind::Newtype);

        let outcome = engine.run_source(
            "assert!(Whim\\Type\\of(1) == Whim\\Type\\id::<int>());\n\
             assert!(Whim\\Type\\to_debug_string(Whim\\Type\\id::<int>()) == 'int');",
            Path::new("/core-type.whim"),
        );
        assert_eq!(outcome.exit_code(), 0);
    }
}
