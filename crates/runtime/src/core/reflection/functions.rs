//! Public reflection entry points.

use core::str;

use whim_macros::whim_function;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::spec::TypeSpec;
use crate::builtin::throw::Throw;
use crate::core::reflection::model::ReflectedType;
use crate::core::reflection::model::ReflectionData;
use crate::core::reflection::objects;
use crate::core::symbols::CHECKED_KIND_ORDER;
use crate::core::symbols::CLASS_LIKE_KIND_ORDER;
use crate::core::symbols::strip_leading_backslash;
use crate::symbols::SymbolKind;
use crate::value::Value;
use crate::value::ValueView;
use crate::value::atom::Atom;
use crate::value::object::TypeEnvironmentId;

#[whim_function(
    "Whim\\Reflection\\get_loaded_symbols(null|Whim\\Symbol\\SymbolKind $kind = null): vec<Whim\\Reflection\\Symbol\\SymbolReflection>",
    must_use
)]
pub(crate) fn get_loaded_symbols(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let kind = arguments
        .get(0)
        .filter(|value| !value.is_null() && !value.is_uninitialized())
        .map(|value| symbol_kind_argument(context, value))
        .transpose()?;
    let mut names = context
        .vm
        .engine
        .tables
        .symbols
        .iter()
        .filter(|(_, entry)| kind.is_none_or(|kind| entry.kind == kind))
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    names.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));

    let mut reflected = Vec::with_capacity(names.len());
    for name in names {
        reflected.push(objects::symbol(context, name)?);
    }

    Ok(context.vec(reflected))
}

#[whim_function(
    "Whim\\Reflection\\reflect_symbol(string $name, bool $autoload = true): null|Whim\\Reflection\\Symbol\\SymbolReflection",
    must_use
)]
pub(crate) fn reflect_symbol(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let name = symbol_name(context, arguments.bytes(0))?;
    let autoload = arguments.get(1).is_none_or(|_| arguments.bool(1));
    reflect_any_symbol(context, name, autoload)
}

#[whim_function(
    "Whim\\Reflection\\reflect_class_like(string|object $class): null|Whim\\Reflection\\Symbol\\ClassLikeReflection",
    must_use
)]
pub(crate) fn reflect_class_like(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let name = class_like_name(context, arguments.get(0).expect("validated argument"))?;
    reflect_expected(context, name, &CLASS_LIKE_KIND_ORDER)
}

#[whim_function(
    "Whim\\Reflection\\reflect_class(string|object $class): null|Whim\\Reflection\\Symbol\\ClassReflection",
    must_use
)]
pub(crate) fn reflect_class(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let name = class_like_name(context, arguments.get(0).expect("validated argument"))?;
    reflect_expected(context, name, &[SymbolKind::Class])
}

#[whim_function(
    "Whim\\Reflection\\reflect_interface(string $interface): null|Whim\\Reflection\\Symbol\\InterfaceReflection",
    must_use
)]
pub(crate) fn reflect_interface(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let name = symbol_name(context, arguments.bytes(0))?;
    reflect_expected(context, name, &[SymbolKind::Interface])
}

#[whim_function(
    "Whim\\Reflection\\reflect_enum(string|object $enum): null|Whim\\Reflection\\Symbol\\EnumReflection",
    must_use
)]
pub(crate) fn reflect_enum(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let name = class_like_name(context, arguments.get(0).expect("validated argument"))?;
    reflect_expected(context, name, &[SymbolKind::Enum])
}

macro_rules! named_reflector {
    ($rust_name:ident, $signature:literal, $kind:ident) => {
        #[whim_function($signature, must_use)]
        pub(crate) fn $rust_name(
            context: &mut Context<'_, '_, '_>,
            arguments: Arguments<'_>,
        ) -> Result<Value, Throw> {
            let name = symbol_name(context, arguments.bytes(0))?;
            reflect_expected(context, name, &[SymbolKind::$kind])
        }
    };
}

named_reflector!(
    reflect_type_alias,
    "Whim\\Reflection\\reflect_type_alias(string $alias): null|Whim\\Reflection\\Symbol\\TypeAliasReflection",
    TypeAlias
);
named_reflector!(
    reflect_newtype,
    "Whim\\Reflection\\reflect_newtype(string $newtype): null|Whim\\Reflection\\Symbol\\NewtypeReflection",
    Newtype
);
named_reflector!(
    reflect_function,
    "Whim\\Reflection\\reflect_function(string $function): null|Whim\\Reflection\\Symbol\\FunctionReflection",
    Function
);
named_reflector!(
    reflect_constant,
    "Whim\\Reflection\\reflect_constant(string $constant): null|Whim\\Reflection\\Symbol\\ConstantReflection",
    Constant
);

#[whim_function(
    "Whim\\Reflection\\reflect_object(object $object): Whim\\Reflection\\ObjectReflection",
    must_use
)]
pub(crate) fn reflect_object(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    objects::build(context, ReflectionData::Object, vec![arguments.local(0)])
}

#[whim_function(
    "Whim\\Reflection\\reflect_callable(fn $callable): Whim\\Reflection\\Callable\\CallableValueReflection",
    must_use
)]
pub(crate) fn reflect_callable(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    objects::build(
        context,
        ReflectionData::CallableValue,
        vec![arguments.local(0)],
    )
}

#[whim_function(
    "Whim\\Reflection\\reflect_newtype_value(mixed $value): null|Whim\\Reflection\\NewtypeValueReflection",
    must_use
)]
pub(crate) fn reflect_newtype_value(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let value = arguments.local(0);
    let Some(identifier) = value.newtype_id() else {
        return Ok(Value::null());
    };
    objects::build(
        context,
        ReflectionData::NewtypeValue(identifier),
        vec![value],
    )
}

#[whim_function(
    "Whim\\Reflection\\reflect_type<T>(): Whim\\Reflection\\Type\\TypeReflection",
    must_use
)]
pub(crate) fn reflect_type(context: &mut Context<'_, '_, '_>) -> Result<Value, Throw> {
    let descriptor = context
        .vm
        .built_in_type_descriptor(&TypeSpec::Parameter("T"), context.type_environment);
    objects::r#type(context, ReflectedType::new(descriptor))
}

#[whim_function(
    "Whim\\Reflection\\reflect_type_of(mixed $value): Whim\\Reflection\\Type\\TypeReflection",
    must_use
)]
pub(crate) fn reflect_type_of(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let descriptor = context
        .vm
        .runtime_type_descriptor(arguments.get(0).expect("validated argument"), 0);
    objects::r#type(context, ReflectedType::new(descriptor))
}

#[whim_function(
    "Whim\\Reflection\\reflect_type_id(Whim\\Type\\TypeId $id): null|Whim\\Reflection\\Type\\TypeReflection",
    must_use
)]
pub(crate) fn reflect_type_id(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
) -> Result<Value, Throw> {
    let Ok(raw) = u32::try_from(arguments.int(0)) else {
        return Ok(Value::null());
    };
    let descriptor = context
        .vm
        .engine
        .tables
        .type_environments
        .get(raw as usize)
        .and_then(|entry| {
            let (name, descriptor) = entry.binding.as_ref()?;
            (entry.parent == Some(TypeEnvironmentId::default())
                && name == &context.vm.engine.tables.type_id_atom)
                .then(|| descriptor.clone())
        });
    let Some(descriptor) = descriptor else {
        return Ok(Value::null());
    };

    objects::r#type(context, ReflectedType::new(descriptor))
}

fn reflect_any_symbol(
    context: &mut Context<'_, '_, '_>,
    name: Atom,
    autoload: bool,
) -> Result<Value, Throw> {
    if context.vm.engine.tables.symbols.contains_key(&name) {
        return objects::symbol(context, name);
    }
    if !autoload {
        return Ok(Value::null());
    }

    for kind in CHECKED_KIND_ORDER {
        if context.vm.run_autoload(kind, name.clone())?
            && context.vm.engine.tables.symbols.contains_key(&name)
        {
            return objects::symbol(context, name);
        }
    }

    Ok(Value::null())
}

fn reflect_expected(
    context: &mut Context<'_, '_, '_>,
    name: Atom,
    kinds: &[SymbolKind],
) -> Result<Value, Throw> {
    if let Some(entry) = context.vm.engine.tables.symbols.get(&name) {
        return if kinds.contains(&entry.kind) {
            objects::symbol(context, name)
        } else {
            Ok(Value::null())
        };
    }

    for kind in kinds {
        context.vm.run_autoload(*kind, name.clone())?;
        if context
            .vm
            .engine
            .tables
            .symbols
            .get(&name)
            .is_some_and(|entry| entry.kind == *kind)
        {
            return objects::symbol(context, name);
        }
    }

    Ok(Value::null())
}

fn class_like_name(context: &mut Context<'_, '_, '_>, value: &Value) -> Result<Atom, Throw> {
    if let Some(object) = value.as_object() {
        return Ok(context.vm.engine.tables.classes[object.class().0 as usize]
            .name
            .clone());
    }
    let Some(bytes) = value.as_string_bytes() else {
        return Err(context.type_error("the reflected class-like must be a string or object"));
    };
    symbol_name(context, bytes)
}

fn symbol_name(context: &mut Context<'_, '_, '_>, bytes: &[u8]) -> Result<Atom, Throw> {
    if str::from_utf8(bytes).is_err() {
        return Err(context.type_error("the reflected symbol name must be valid UTF-8"));
    }
    let name = context.vm.intern(bytes);
    Ok(strip_leading_backslash(context.vm.heap(), name))
}

fn symbol_kind_argument(
    context: &mut Context<'_, '_, '_>,
    value: &Value,
) -> Result<SymbolKind, Throw> {
    let Some(instance) = value.as_object() else {
        return Err(context.type_error("the symbol kind must be an enum case"));
    };
    let value_name = context.vm.intern(b"value");
    let class = &context.vm.engine.tables.classes[instance.class().0 as usize];
    let Some(slot) = class.slot_names.get(&value_name).copied() else {
        return Err(context.type_error("the symbol kind must be a backed enum case"));
    };
    let backing = instance.read_slot(slot as usize);
    let ValueView::Int(number) = backing.transparent() else {
        return Err(context.type_error("the symbol kind must have an integer backing value"));
    };
    let kind = match *number {
        value if value == SymbolKind::Class as i64 => SymbolKind::Class,
        value if value == SymbolKind::Interface as i64 => SymbolKind::Interface,
        value if value == SymbolKind::Enum as i64 => SymbolKind::Enum,
        value if value == SymbolKind::TypeAlias as i64 => SymbolKind::TypeAlias,
        value if value == SymbolKind::Newtype as i64 => SymbolKind::Newtype,
        value if value == SymbolKind::Function as i64 => SymbolKind::Function,
        value if value == SymbolKind::Constant as i64 => SymbolKind::Constant,
        _ => return Err(context.type_error("the symbol kind backing value is not recognized")),
    };
    Ok(kind)
}
