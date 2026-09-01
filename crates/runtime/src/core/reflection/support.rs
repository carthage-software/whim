//! Shared declaration and type metadata lookup.

use std::ptr::NonNull;
use std::rc::Rc;

use whim_span::Span;

use crate::builtin::throw::Throw;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::BuiltInCallableAttributes;
use crate::bytecode::unit::CompiledAttribute;
use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::CompiledParameter;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::ConstantInitializer;
use crate::bytecode::unit::MUST_USE_ATTRIBUTE;
use crate::bytecode::unit::TRACE_BOUNDARY_ATTRIBUTE;
use crate::bytecode::unit::TRACK_CALLER_ATTRIBUTE;
use crate::bytecode::unit::literal_value;
use crate::classes::MethodBodyKind;
use crate::core::reflection::model::CallableKey;
use crate::core::reflection::model::DeclarationKey;
use crate::core::reflection::model::DeclarationMetadata;
use crate::core::reflection::model::GenericOwner;
use crate::core::reflection::model::MemberKey;
use crate::core::reflection::model::MemberKind;
use crate::engine::builtins::built_in_parameters;
use crate::engine::builtins::built_in_type_parameters;
use crate::linker::descriptors::descriptor_from_built_in_spec;
use crate::symbols::FunctionLocator;
use crate::symbols::FunctionTable;
use crate::symbols::RuntimeFunction;
use crate::symbols::SymbolKind;
use crate::symbols::UnitContext;
use crate::value::Value;
use crate::value::atom::Atom;
use crate::value::function::FuncId;
use crate::value::object::ClassId;
use crate::vm::VirtualMachine;

#[derive(Clone)]
pub(crate) struct CallableInfo {
    pub(crate) name: Atom,
    pub(crate) parameters: Vec<CompiledParameter>,
    pub(crate) type_parameters: Vec<CompiledTypeParameter>,
    pub(crate) return_type: Option<TypeDescriptor>,
    pub(crate) attributes: Vec<CompiledAttribute>,
    pub(crate) unit: Option<Rc<UnitContext>>,
    pub(crate) span: Option<Span>,
    pub(crate) captures_this: bool,
    pub(crate) capture_names: Vec<Atom>,
    pub(crate) is_short_closure: bool,
    pub(crate) declaring_class: Option<ClassId>,
}

impl CallableInfo {
    pub(crate) fn required_parameters(&self) -> usize {
        self.parameters
            .iter()
            .filter(|parameter| !parameter.has_default)
            .count()
    }
}

pub(crate) fn callable_info(vm: &VirtualMachine<'_>, key: &CallableKey) -> Option<CallableInfo> {
    match key {
        CallableKey::Function(name) => function_info(vm, name),
        CallableKey::Method { class, name } => method_info(vm, *class, name),
        CallableKey::Closure(function) => user_function_info(vm, *function, None),
    }
}

fn function_info(vm: &VirtualMachine<'_>, name: &Atom) -> Option<CallableInfo> {
    let entry = vm.engine.tables.symbols.get(name)?;
    if entry.kind != SymbolKind::Function {
        return None;
    }
    match entry.table {
        FunctionTable::User => user_function_info(vm, FuncId(entry.index), None),
        FunctionTable::BuiltIn => {
            let declaration = vm
                .engine
                .tables
                .built_in_function_declarations
                .get(entry.index as usize)?;
            Some(CallableInfo {
                name: declaration.name.clone(),
                parameters: declaration.parameters.clone(),
                type_parameters: declaration.type_parameters.clone(),
                return_type: Some(declaration.return_type.clone()),
                attributes: built_in_attributes(vm, declaration.attributes),
                unit: None,
                span: None,
                captures_this: false,
                capture_names: Vec::new(),
                is_short_closure: false,
                declaring_class: None,
            })
        }
    }
}

fn method_info(vm: &VirtualMachine<'_>, class: ClassId, name: &Atom) -> Option<CallableInfo> {
    let entry = vm.engine.tables.classes[class.0 as usize].method(name)?;
    match entry.body {
        MethodBodyKind::Bytecode(function) => user_function_info(vm, function, Some(name.clone())),
        MethodBodyKind::BuiltIn(body) => Some(CallableInfo {
            name: name.clone(),
            parameters: built_in_parameters(vm.heap(), body.parameters),
            type_parameters: built_in_type_parameters(vm.heap(), body.type_parameters),
            return_type: Some(descriptor_from_built_in_spec(vm.heap(), &body.return_spec)),
            attributes: built_in_attributes(vm, body.attributes),
            unit: None,
            span: None,
            captures_this: !entry.is_static,
            capture_names: Vec::new(),
            is_short_closure: false,
            declaring_class: Some(entry.declaring_class),
        }),
    }
}

fn user_function_info(
    vm: &VirtualMachine<'_>,
    function: FuncId,
    name: Option<Atom>,
) -> Option<CallableInfo> {
    let runtime = vm.engine.tables.functions.get(function.0 as usize)?;
    let compiled = compiled_function(runtime)?;
    Some(CallableInfo {
        name: name.unwrap_or_else(|| runtime.name.clone()),
        parameters: runtime.parameters().to_vec(),
        type_parameters: runtime.type_parameters().to_vec(),
        return_type: runtime.return_type.as_deref().cloned(),
        attributes: runtime.attributes().to_vec(),
        unit: Some(Rc::clone(&runtime.unit)),
        span: Some(compiled.span),
        captures_this: runtime.captures_this,
        capture_names: compiled.capture_names.clone(),
        is_short_closure: compiled.is_short_closure,
        declaring_class: runtime.declaring_class,
    })
}

pub(crate) fn compiled_function(runtime: &RuntimeFunction) -> Option<&CompiledFunction> {
    match runtime.locator {
        FunctionLocator::TopLevel(position) => runtime.unit.unit.functions.get(position as usize),
        FunctionLocator::Method { class, method } => runtime
            .unit
            .unit
            .classes
            .get(class as usize)?
            .methods
            .get(method as usize)
            .map(|method| &method.function),
    }
}

pub(crate) fn type_parameters(
    vm: &VirtualMachine<'_>,
    owner: &GenericOwner,
) -> Option<Vec<CompiledTypeParameter>> {
    match owner {
        GenericOwner::Callable(callable) => Some(callable_info(vm, callable)?.type_parameters),
        GenericOwner::Symbol(name) => {
            let entry = vm.engine.tables.symbols.get(name)?;
            Some(match entry.kind {
                SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum => vm
                    .engine
                    .tables
                    .classes
                    .get(entry.index as usize)?
                    .type_parameters
                    .to_vec(),
                SymbolKind::TypeAlias => vm
                    .engine
                    .tables
                    .type_aliases
                    .get(entry.index as usize)?
                    .type_parameters
                    .clone(),
                SymbolKind::Newtype => vm
                    .engine
                    .tables
                    .newtypes
                    .get(entry.index as usize)?
                    .type_parameters
                    .clone(),
                SymbolKind::Function => {
                    callable_info(vm, &CallableKey::Function(name.clone()))?.type_parameters
                }
                SymbolKind::Constant => Vec::new(),
            })
        }
    }
}

pub(crate) fn generic_owner_unit(
    vm: &VirtualMachine<'_>,
    owner: &GenericOwner,
) -> Option<Rc<UnitContext>> {
    match owner {
        GenericOwner::Callable(callable) => callable_info(vm, callable)?.unit,
        GenericOwner::Symbol(name) => symbol_metadata(vm, name).unit,
    }
}

pub(crate) fn declaration_metadata(
    vm: &VirtualMachine<'_>,
    declaration: &DeclarationKey,
) -> DeclarationMetadata {
    match declaration {
        DeclarationKey::Symbol(name) => symbol_metadata(vm, name),
        DeclarationKey::Member(member) => member_metadata(vm, member),
        DeclarationKey::Parameter { callable, position } => {
            let Some(info) = callable_info(vm, callable) else {
                return empty_metadata();
            };
            let Some(parameter) = info.parameters.get(*position) else {
                return empty_metadata();
            };
            DeclarationMetadata {
                unit: info.unit,
                span: (parameter.span != Span::zero()).then_some(parameter.span),
                attributes: parameter.attributes.clone(),
            }
        }
        DeclarationKey::Closure(function) => callable_info(vm, &CallableKey::Closure(*function))
            .map_or_else(empty_metadata, |info| DeclarationMetadata {
                unit: info.unit,
                span: info.span,
                attributes: info.attributes,
            }),
    }
}

fn symbol_metadata(vm: &VirtualMachine<'_>, name: &Atom) -> DeclarationMetadata {
    let Some(entry) = vm.engine.tables.symbols.get(name) else {
        return empty_metadata();
    };
    match entry.kind {
        SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum => {
            let class = &vm.engine.tables.classes[entry.index as usize];
            let unit = class.attribute_unit.clone();
            let span = unit
                .as_ref()
                .and_then(|unit| compiled_class(unit, name))
                .map(|class| class.span);
            DeclarationMetadata {
                unit,
                span,
                attributes: class.attribute_declarations.clone(),
            }
        }
        SymbolKind::Function => callable_info(vm, &CallableKey::Function(name.clone()))
            .map_or_else(empty_metadata, |info| DeclarationMetadata {
                unit: info.unit,
                span: info.span,
                attributes: info.attributes,
            }),
        SymbolKind::TypeAlias => vm
            .engine
            .tables
            .type_aliases
            .get(entry.index as usize)
            .map_or_else(empty_metadata, |alias| {
                metadata_from_units(vm, name, alias.span, alias.attributes.clone())
            }),
        SymbolKind::Newtype => vm
            .engine
            .tables
            .newtypes
            .get(entry.index as usize)
            .map_or_else(empty_metadata, |newtype| {
                metadata_from_units(vm, name, newtype.span, newtype.attributes.clone())
            }),
        SymbolKind::Constant => {
            for unit in vm.engine.units.iter().rev() {
                if let Some(constant) = unit.unit.constants.iter().find(|value| value.name == *name)
                {
                    return DeclarationMetadata {
                        unit: Some(Rc::clone(unit)),
                        span: Some(constant.span),
                        attributes: constant.attributes.clone(),
                    };
                }
            }
            empty_metadata()
        }
    }
}

fn member_metadata(vm: &VirtualMachine<'_>, member: &MemberKey) -> DeclarationMetadata {
    let class = &vm.engine.tables.classes[member.class.0 as usize];
    let Some(unit) = class.attribute_unit.as_ref() else {
        return empty_metadata();
    };
    let Some(compiled) = compiled_class(unit, &class.name) else {
        return empty_metadata();
    };
    let (span, attributes) = match member.kind {
        MemberKind::Method => compiled
            .methods
            .iter()
            .find(|value| value.name == member.name)
            .map(|value| (value.function.span, value.function.attributes.clone())),
        MemberKind::Property => compiled
            .properties
            .iter()
            .find(|value| value.name == member.name)
            .map(|value| (value.span, value.attributes.clone())),
        MemberKind::ClassConstant => compiled
            .constants
            .iter()
            .find(|value| value.name == member.name)
            .map(|value| (value.span, value.attributes.clone())),
        MemberKind::EnumCase => compiled
            .cases
            .iter()
            .find(|value| value.name == member.name)
            .map(|value| (value.span, value.attributes.clone())),
    }
    .map_or((None, Vec::new()), |(span, attributes)| {
        (Some(span), attributes)
    });
    DeclarationMetadata {
        unit: Some(Rc::clone(unit)),
        span,
        attributes,
    }
}

pub(crate) fn compiled_class<'a>(
    unit: &'a UnitContext,
    name: &Atom,
) -> Option<&'a CompiledClassLike> {
    unit.unit.classes.iter().find(|class| class.name == *name)
}

fn metadata_from_units(
    vm: &VirtualMachine<'_>,
    name: &Atom,
    span: Span,
    attributes: Vec<CompiledAttribute>,
) -> DeclarationMetadata {
    let unit = vm.engine.units.iter().rev().find(|unit| {
        unit.unit
            .type_aliases
            .iter()
            .any(|value| value.name == *name)
            || unit.unit.newtypes.iter().any(|value| value.name == *name)
    });
    DeclarationMetadata {
        unit: unit.cloned(),
        span: Some(span),
        attributes,
    }
}

const fn empty_metadata() -> DeclarationMetadata {
    DeclarationMetadata {
        unit: None,
        span: None,
        attributes: Vec::new(),
    }
}

fn built_in_attributes(
    vm: &VirtualMachine<'_>,
    attributes: BuiltInCallableAttributes,
) -> Vec<CompiledAttribute> {
    let mut declarations = Vec::with_capacity(3);
    for (present, name) in [
        (attributes.track_caller, TRACK_CALLER_ATTRIBUTE),
        (attributes.trace_boundary, TRACE_BOUNDARY_ATTRIBUTE),
        (attributes.must_use, MUST_USE_ATTRIBUTE),
    ] {
        if present {
            declarations.push(CompiledAttribute {
                class: vm.intern(name),
                span: Span::zero(),
                arguments: Vec::new(),
                named_arguments: Vec::new(),
            });
        }
    }
    declarations
}

pub(crate) fn evaluate_initializer(
    vm: &mut VirtualMachine<'_>,
    initializer: &ConstantInitializer,
    unit: Option<&Rc<UnitContext>>,
) -> Result<Value, Throw> {
    match initializer {
        ConstantInitializer::Literal(literal) => Ok(literal_value(literal)),
        ConstantInitializer::Thunk(chunk) => {
            let Some(unit) = unit else {
                return Err(vm.throw_well_known_value(
                    vm.engine.tables.well_known.type_error,
                    "a core reflection initializer cannot contain bytecode".to_string(),
                ));
            };
            vm.run_initializer(NonNull::from(&**chunk), unit)
                .map_err(|control| vm.control_to_throw(control))
        }
    }
}
