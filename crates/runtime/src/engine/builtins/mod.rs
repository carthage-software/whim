//! Declaring the Rust-backed symbols every engine requires.

use crate::builtin::spec::ConstantValue;
use crate::builtin::spec::CoreDeclarations;
use crate::builtin::spec::FunctionSpec;
use crate::builtin::spec::ParameterDefaultSpec;
use crate::builtin::spec::ParameterSpec;
use crate::builtin::spec::TypeParameterSpec;
use crate::bytecode::REFERENCE_REGISTER_LIMIT;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::BuiltInCallableAttributes;
use crate::bytecode::unit::CompiledBuiltInFunction;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::CompiledNewtype;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::ConstantInitializer;
use crate::bytecode::unit::frameless_literal;
use crate::classes::BuiltInMethodBody;
use crate::engine::Atom;
use crate::engine::ClassId;
use crate::engine::CompiledParameter;
use crate::engine::Engine;
use crate::engine::FunctionLocator;
use crate::engine::FunctionTable;
use crate::engine::Heap;
use crate::engine::InlineCache;
use crate::engine::NonNull;
use crate::engine::Rc;
use crate::engine::SymbolKind;
use crate::engine::UnitContext;
use crate::engine::Value;
use crate::engine::declare::ConstantSlot;
use crate::engine::descriptor_from_built_in_spec;
use crate::engine::tables::RuntimeTables;
use crate::symbols::CallableOptimization;
use crate::symbols::RuntimeFunction;
use crate::symbols::SymbolEntry;
use crate::u32_index;
use crate::unwrap_result_invariant;
use crate::value::ValueView;
use crate::value::function::BuiltInId;

mod declarations;

#[derive(Clone)]
pub(crate) enum BuiltInCallable {
    /// A built-in function; the spec is owned because a generic handler cannot
    /// live in a `static`.
    Function(FunctionSpec),
    Method {
        body: BuiltInMethodBody,
        name: Atom,
    },
}

impl BuiltInCallable {
    pub(crate) const fn parameters(&self) -> &'static [ParameterSpec] {
        match self {
            Self::Function(spec) => spec.parameters,
            Self::Method { body, .. } => body.parameters,
        }
    }

    pub(crate) const fn type_parameters(&self) -> &'static [TypeParameterSpec] {
        match self {
            Self::Function(spec) => spec.type_parameters,
            Self::Method { body, .. } => body.type_parameters,
        }
    }

    pub(crate) fn display_name(&self) -> String {
        match self {
            Self::Function(spec) => spec.name.to_string(),
            Self::Method { name, .. } => name.to_string_lossy().into_owned(),
        }
    }
}

pub(crate) fn runtime_function(
    function: &CompiledFunction,
    context: &Rc<UnitContext>,
    locator: FunctionLocator,
    declaring_class: Option<ClassId>,
) -> RuntimeFunction {
    let (required, declared) = arity_of(&function.parameters);
    let reference_parameter_mask = function
        .parameters
        .iter()
        .take(usize::from(REFERENCE_REGISTER_LIMIT))
        .enumerate()
        .fold(0u64, |mask, (position, parameter)| {
            if parameter
                .declared_type
                .as_ref()
                .is_none_or(TypeDescriptor::may_hold_reference)
            {
                mask | (1u64 << position)
            } else {
                mask
            }
        });
    RuntimeFunction {
        name: function.name.clone(),
        signature: function.signature.clone(),
        unit: Rc::clone(context),
        locator,
        chunk: NonNull::from(&function.chunk),
        optimized_chunk: None,
        optimization: if context.lazy_callables {
            CallableOptimization::Pending
        } else {
            CallableOptimization::Complete
        },
        parameters: NonNull::from(function.parameters.as_slice()),
        type_parameters: NonNull::from(function.type_parameters.as_slice()),
        attributes: NonNull::from(function.attributes.as_slice()),
        frameless_literal: frameless_literal(function),
        return_type: function.return_type.clone().map(Box::new),
        captures_this: function.captures_this,
        declaring_class,
        required_parameters: required,
        declared_parameters: declared,
        reference_parameter_mask,
        cache: Box::new(InlineCache::new()),
    }
}

/// The arity shape of a parameter list: how many parameters must be given and
/// how many are declared.
fn arity_of(parameters: &[CompiledParameter]) -> (u8, u8) {
    let declared = u8_arity(parameters.len());
    let required = u8_arity(
        parameters
            .iter()
            .filter(|parameter| !parameter.has_default)
            .count(),
    );
    (required, declared)
}

fn u8_arity(value: usize) -> u8 {
    // SAFETY: the surrounding invariant proves this result is successful.
    unsafe {
        unwrap_result_invariant(
            u8::try_from(value),
            "a callable cannot declare more than u8::MAX parameters",
        )
    }
}

fn constant_spec_value(heap: &Heap, value: &ConstantValue) -> Value {
    match value {
        ConstantValue::Bool(value) => Value::bool(*value),
        ConstantValue::Int(value) => Value::int(*value),
        ConstantValue::Float(value) => Value::float(*value),
        ConstantValue::String(value) => Value::string(heap.intern(value.as_bytes()).to_handle()),
    }
}

pub(crate) fn built_in_type_parameters(
    heap: &Heap,
    parameters: &[TypeParameterSpec],
) -> Vec<CompiledTypeParameter> {
    parameters
        .iter()
        .map(|parameter| CompiledTypeParameter {
            name: heap.intern(parameter.name.as_bytes()),
            span: whim_span::Span::zero(),
            variance: parameter.variance,
            bounds: parameter
                .bounds
                .iter()
                .map(|bound| descriptor_from_built_in_spec(heap, bound))
                .collect(),
            default: parameter
                .default
                .as_ref()
                .map(|default| descriptor_from_built_in_spec(heap, default)),
        })
        .collect()
}

pub(crate) fn built_in_parameters(
    heap: &Heap,
    parameters: &[ParameterSpec],
) -> Vec<CompiledParameter> {
    parameters
        .iter()
        .map(|parameter| CompiledParameter {
            name: heap.intern(parameter.name.as_bytes()),
            span: whim_span::Span::zero(),
            has_default: parameter.optional,
            default: parameter
                .default
                .as_ref()
                .map(|default| ConstantInitializer::Literal(parameter_default(heap, default))),
            declared_type: Some(descriptor_from_built_in_spec(heap, &parameter.type_spec)),
            sensitive: parameter.sensitive,
            attributes: Vec::new(),
        })
        .collect()
}

fn parameter_default(heap: &Heap, default: &ParameterDefaultSpec) -> Literal {
    match default {
        ParameterDefaultSpec::Null => Literal::Null,
        ParameterDefaultSpec::Bool(value) => Literal::Bool(*value),
        ParameterDefaultSpec::Int(value) => Literal::Int(*value),
        ParameterDefaultSpec::Float(value) => Literal::Float(*value),
        ParameterDefaultSpec::String(value) => Literal::String(heap.intern(value)),
    }
}

pub(in crate::engine) fn text_of(value: &Value) -> String {
    if value.newtype_id().is_some() {
        return value.kind_name().to_string();
    }

    match value.transparent() {
        ValueView::String(_) | ValueView::ShortString(_) => {
            // SAFETY: both matched views are strings.
            String::from_utf8_lossy(unsafe { value.as_string_bytes().unwrap_unchecked() })
                .into_owned()
        }
        ValueView::Int(value) => value.to_string(),
        ValueView::Float(value) => value.to_string(),
        _ => value.kind_name().to_string(),
    }
}

/// The fewest and most type arguments a reference to a declaration with these
/// type parameters may supply: everything without a default is required, and
/// the total is all of them.
pub(crate) fn binder_arity_of(type_parameters: &[CompiledTypeParameter]) -> (u32, u32) {
    let total = u32_index(type_parameters.len());
    let required = type_parameters
        .iter()
        .position(|parameter| parameter.default.is_some())
        .map_or_else(|| total, u32_index);
    (required, total)
}

impl Engine {
    pub(crate) fn intern_built_in_function(&mut self, spec: FunctionSpec) -> BuiltInId {
        let key = (spec.handler as usize, spec.signature);
        if let Some(id) = self.tables.built_in_function_ids.get(&key) {
            return *id;
        }

        // SAFETY: the surrounding invariant proves this result is successful.
        let id = BuiltInId(unsafe {
            unwrap_result_invariant(
                u32::try_from(self.tables.built_in_functions.len()),
                "the built-in callable table fits in a BuiltInId",
            )
        });
        self.tables
            .built_in_functions
            .push(BuiltInCallable::Function(spec));
        self.tables.built_in_function_ids.insert(key, id);
        id
    }

    pub(crate) fn intern_built_in_method(
        &mut self,
        declaring_class: ClassId,
        name: Atom,
        body: BuiltInMethodBody,
    ) -> BuiltInId {
        let key = (declaring_class, name.clone());
        if let Some(id) = self.tables.built_in_method_ids.get(&key) {
            return *id;
        }

        // SAFETY: the surrounding invariant proves this result is successful.
        let id = BuiltInId(unsafe {
            unwrap_result_invariant(
                u32::try_from(self.tables.built_in_functions.len()),
                "the built-in callable table fits in a BuiltInId",
            )
        });
        self.tables
            .built_in_functions
            .push(BuiltInCallable::Method { body, name });
        self.tables.built_in_method_ids.insert(key, id);
        id
    }
}

impl RuntimeTables {
    pub(crate) fn register_core(&mut self, heap: &Heap, core: &CoreDeclarations) {
        for spec in &core.newtypes {
            let name = heap.intern(spec.name.as_bytes());
            self.check_core_name(&name);
            let index = u32_index(self.newtypes.len());
            self.newtypes.push(CompiledNewtype {
                name: name.clone(),
                span: whim_span::Span::zero(),
                attributes: Vec::new(),
                type_parameters: built_in_type_parameters(heap, spec.type_parameters),
                backing: descriptor_from_built_in_spec(heap, &spec.backing),
            });
            self.symbols.insert(
                name,
                SymbolEntry {
                    kind: SymbolKind::Newtype,
                    index,
                    table: FunctionTable::BuiltIn,
                },
            );
        }
        for declaration in &core.functions {
            let spec = &declaration.callable;
            let name = heap.intern(spec.name.as_bytes());
            self.check_core_name(&name);
            let index = u32_index(self.built_in_functions.len());
            self.built_in_functions
                .push(BuiltInCallable::Function(*spec));
            self.built_in_function_declarations
                .push(CompiledBuiltInFunction {
                    name: name.clone(),
                    type_parameters: built_in_type_parameters(heap, spec.type_parameters),
                    parameters: built_in_parameters(heap, spec.parameters),
                    return_type: descriptor_from_built_in_spec(heap, &spec.return_spec),
                    attributes: BuiltInCallableAttributes::resolve(spec.name, declaration.markers),
                });
            self.symbols.insert(
                name,
                SymbolEntry {
                    kind: SymbolKind::Function,
                    index,
                    table: FunctionTable::BuiltIn,
                },
            );
        }
        for spec in &core.interfaces {
            self.register_builtin_interface(heap, spec);
        }
        for spec in &core.classes {
            self.register_builtin_class(heap, spec);
        }
        for spec in &core.enums {
            self.register_builtin_enum(heap, spec);
        }
        for spec in core.constants {
            let name = heap.intern(spec.name.as_bytes());
            self.check_core_name(&name);
            let index = u32_index(self.constants.len());
            self.constants
                .push(ConstantSlot::Evaluated(constant_spec_value(
                    heap,
                    &spec.value,
                )));
            self.symbols.insert(
                name,
                SymbolEntry {
                    kind: SymbolKind::Constant,
                    index,
                    table: FunctionTable::User,
                },
            );
        }
    }

    fn check_core_name(&self, name: &Atom) {
        assert!(
            !self.symbols.contains_key(name),
            "the generated core declares {} more than once",
            name.to_string_lossy()
        );
    }
}
