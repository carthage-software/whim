//! Checking a value against a declared type descriptor.

use std::hash::BuildHasher;
use std::hash::Hash;
use std::hash::Hasher;

use foldhash::fast::FixedState;
use std::mem::discriminant;

use crate::bytecode::chunk::descriptors::FunctionTypeDescriptor;
use crate::bytecode::chunk::descriptors::FunctionTypeParameterDescriptor;
use crate::bytecode::render;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::Variance;
use crate::engine::builtins::BuiltInCallable;
use crate::engine::builtins::built_in_type_parameters;
use crate::limits::MAX_TYPE_DEPTH_U32;
use crate::linker::descriptors::descriptor_from_built_in_spec;
use crate::symbols::RuntimeTypeEnvironment;
use crate::value::ValueView;
use crate::value::array::ArrayTypeCheckId;
use crate::value::function::PresetArg;
use crate::vm::Atom;
use crate::vm::CallTarget;
use crate::vm::Chunk;
use crate::vm::ClassId;
use crate::vm::DescriptorIndex;
use crate::vm::FunctionObject;
use crate::vm::Heap;
use crate::vm::Key;
use crate::vm::KeyRef;
use crate::vm::ManagedRef;
use crate::vm::SymbolKind;
use crate::vm::TypeDescriptor;
use crate::vm::TypeEnvironmentId;
use crate::vm::Value;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;
use crate::vm::is_instance_of;
use crate::vm::ops;
use crate::vm::unreachable_invariant;

fn array_type_check_cacheable(descriptor: &TypeDescriptor) -> bool {
    match descriptor {
        TypeDescriptor::Wildcard
        | TypeDescriptor::Mixed
        | TypeDescriptor::Null
        | TypeDescriptor::Bool
        | TypeDescriptor::Int
        | TypeDescriptor::Float
        | TypeDescriptor::String
        | TypeDescriptor::Object
        | TypeDescriptor::TrueLiteral
        | TypeDescriptor::FalseLiteral
        | TypeDescriptor::IntLiteral(_)
        | TypeDescriptor::IntRange { .. }
        | TypeDescriptor::FloatLiteral(_)
        | TypeDescriptor::Array(None)
        | TypeDescriptor::Vector(None)
        | TypeDescriptor::Dictionary(None)
        | TypeDescriptor::TupleAny => true,
        TypeDescriptor::Array(Some((key, value)))
        | TypeDescriptor::Dictionary(Some((key, value))) => {
            array_type_check_cacheable(key) && array_type_check_cacheable(value)
        }
        TypeDescriptor::Vector(Some(element)) | TypeDescriptor::Negated(element) => {
            array_type_check_cacheable(element)
        }
        TypeDescriptor::Tuple(members)
        | TypeDescriptor::Union(members)
        | TypeDescriptor::Intersection(members) => members.iter().all(array_type_check_cacheable),
        TypeDescriptor::TupleRest { elements, rest } => {
            elements.iter().all(array_type_check_cacheable) && array_type_check_cacheable(rest)
        }
        TypeDescriptor::Named { arguments, .. } => arguments
            .as_ref()
            .is_none_or(|arguments| arguments.iter().all(array_type_check_cacheable)),
        TypeDescriptor::Void
        | TypeDescriptor::Never
        | TypeDescriptor::StringLiteral(_)
        | TypeDescriptor::Member { .. }
        | TypeDescriptor::Parameter(_)
        | TypeDescriptor::StaticClass
        | TypeDescriptor::Callable(_)
        | TypeDescriptor::Classname(_)
        | TypeDescriptor::VectorShape { .. }
        | TypeDescriptor::DictionaryShape { .. } => false,
    }
}

mod check;
mod environment;
mod parser;

pub(crate) use environment::descriptor_same;

use crate::vm::types::parser::RuntimeTypeParser;
use crate::vm::types::parser::push_runtime_union_member;
use crate::vm::types::parser::runtime_union;

impl VirtualMachine<'_> {
    pub(in crate::vm) fn array_type_check_id(
        &mut self,
        descriptor: &TypeDescriptor,
    ) -> Option<ArrayTypeCheckId> {
        if !array_type_check_cacheable(descriptor) {
            return None;
        }

        let identifier = self.intern_type_descriptor_ref(descriptor);
        Some(ArrayTypeCheckId::new(identifier.0))
    }

    /// Parses a runtime type-name spelling.
    pub(in crate::vm) fn parse_runtime_type_name(&self, bytes: &[u8]) -> Option<TypeDescriptor> {
        RuntimeTypeParser::new(&self.heap, bytes).parse()
    }

    fn call_target_type_descriptor(
        &self,
        target: CallTarget,
        environment: TypeEnvironmentId,
    ) -> FunctionTypeDescriptor {
        let (parameters, return_type): (Vec<FunctionTypeParameterDescriptor>, TypeDescriptor) =
            match target {
                CallTarget::User(id) => {
                    let runtime = &self.engine.tables.functions[id.0 as usize];
                    (
                        runtime
                            .parameters()
                            .iter()
                            .map(|parameter| FunctionTypeParameterDescriptor {
                                r#type: parameter.declared_type.as_ref().map_or(
                                    TypeDescriptor::Mixed,
                                    |descriptor| {
                                        self.substitute_descriptor(descriptor, environment, 0)
                                    },
                                ),
                                optional: parameter.has_default,
                            })
                            .collect(),
                        runtime
                            .return_type
                            .as_ref()
                            .map_or(TypeDescriptor::Mixed, |descriptor| {
                                self.substitute_descriptor(descriptor, environment, 0)
                            }),
                    )
                }
                CallTarget::BuiltIn(id) => {
                    match self.engine.tables.built_in_functions[id.0 as usize] {
                        BuiltInCallable::Function(spec) => (
                            spec.parameters
                                .iter()
                                .map(|parameter| FunctionTypeParameterDescriptor {
                                    r#type: self.substitute_descriptor(
                                        &descriptor_from_built_in_spec(
                                            &self.heap,
                                            &parameter.type_spec,
                                        ),
                                        environment,
                                        0,
                                    ),
                                    optional: parameter.optional,
                                })
                                .collect(),
                            self.substitute_descriptor(
                                &descriptor_from_built_in_spec(&self.heap, &spec.return_spec),
                                environment,
                                0,
                            ),
                        ),
                        BuiltInCallable::Method { body, .. } => (
                            body.parameters
                                .iter()
                                .map(|parameter| FunctionTypeParameterDescriptor {
                                    r#type: self.substitute_descriptor(
                                        &descriptor_from_built_in_spec(
                                            &self.heap,
                                            &parameter.type_spec,
                                        ),
                                        environment,
                                        0,
                                    ),
                                    optional: parameter.optional,
                                })
                                .collect(),
                            self.substitute_descriptor(
                                &descriptor_from_built_in_spec(&self.heap, &body.return_spec),
                                environment,
                                0,
                            ),
                        ),
                    }
                }
            };
        FunctionTypeDescriptor {
            parameters,
            return_type: Box::new(return_type),
        }
    }

    /// Reconstructs a callable value's remaining parameter and return shape,
    /// applying specialization and removing parameters fixed by a partial.
    fn callable_type_descriptor(
        &self,
        function: &ManagedRef<FunctionObject>,
        environment: TypeEnvironmentId,
    ) -> FunctionTypeDescriptor {
        let descriptor = self.call_target_type_descriptor(function.target(), environment);
        let mut holes = Vec::new();
        let mut remaining = Vec::new();
        let has_presets = !function.presets().is_empty();
        for (position, parameter) in descriptor.parameters.into_iter().enumerate() {
            match function.presets().get(position) {
                Some(PresetArg::Given(_)) => {}
                Some(PresetArg::Hole(order)) => holes.push((*order, parameter)),
                _ if !has_presets => remaining.push(parameter),
                _ => {}
            }
        }
        holes.sort_unstable_by_key(|(order, _)| *order);
        let parameters = holes
            .into_iter()
            .map(|(_, parameter)| parameter)
            .chain(remaining)
            .collect();
        FunctionTypeDescriptor {
            parameters,
            return_type: descriptor.return_type,
        }
    }

    /// Resolves an unbound callable's defaults when every own type parameter
    /// has one. Such a callable can be invoked without a turbofish, so its
    /// default specialization is also the shape used at a callable boundary.
    fn defaulted_callable_type_environment(
        &mut self,
        function: &ManagedRef<FunctionObject>,
    ) -> Result<TypeEnvironmentId, VirtualMachineControl> {
        let outer = function.type_environment();
        if function.type_arguments_bound() {
            return Ok(outer);
        }

        match function.target() {
            CallTarget::User(id) => {
                let (parameters, subject) = {
                    let runtime = &self.engine.tables.functions[id.0 as usize];
                    (runtime.type_parameters, runtime.name.clone())
                };
                // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
                let parameters = unsafe { parameters.as_ref() };
                if parameters.is_empty()
                    || parameters
                        .iter()
                        .any(|parameter| parameter.default.is_none())
                {
                    return Ok(outer);
                }

                self.bind_type_parameters(parameters, None, outer, subject.as_bytes())
            }
            CallTarget::BuiltIn(id) => {
                let callable = self.engine.tables.built_in_functions[id.0 as usize].clone();
                let parameters = built_in_type_parameters(&self.heap, callable.type_parameters());
                if parameters.is_empty()
                    || parameters
                        .iter()
                        .any(|parameter| parameter.default.is_none())
                {
                    return Ok(outer);
                }

                self.bind_type_parameters(
                    &parameters,
                    None,
                    outer,
                    callable.display_name().as_bytes(),
                )
            }
        }
    }

    /// The language-level type name of a value, for diagnostics: complete
    /// callable signatures, class names for objects, and the runtime kind
    /// otherwise.
    pub(in crate::vm) fn value_type_name(&self, value: &Value) -> String {
        if value.is_function() || value.newtype_id().is_some() {
            return self.runtime_type_name(value);
        }

        match value.transparent() {
            ValueView::Object(instance) => self.value_class_name(instance),
            _ => value.kind_name().to_string(),
        }
    }

    /// Reconstructs the complete runtime type carried by a value. array
    /// types describe their current contents; empty positions use `never`,
    /// the bottom type. `_` is only a written existential pattern and is
    /// never produced by runtime type reconstruction.
    pub(crate) fn runtime_type_descriptor(&self, value: &Value, depth: u32) -> TypeDescriptor {
        if depth > MAX_TYPE_DEPTH_U32 {
            return TypeDescriptor::Mixed;
        }
        if let Some(id) = value.newtype_id() {
            let tagged = self.engine.tables.newtype_value(id);
            let declaration = &self.engine.tables.newtypes[tagged.declaration.0 as usize];
            let arguments = (!declaration.type_parameters.is_empty()).then(|| {
                declaration
                    .type_parameters
                    .iter()
                    .map(|parameter| {
                        self.type_environment_binding(tagged.type_environment, &parameter.name)
                            .cloned()
                            .unwrap_or(TypeDescriptor::Mixed)
                    })
                    .collect()
            });
            return TypeDescriptor::Named {
                name: declaration.name.clone(),
                arguments,
                recursive: false,
            };
        }

        match value.transparent() {
            ValueView::Uninitialized => TypeDescriptor::Never,
            ValueView::Null => TypeDescriptor::Null,
            ValueView::Bool(_) => TypeDescriptor::Bool,
            ValueView::Int(_) => TypeDescriptor::Int,
            ValueView::Float(_) => TypeDescriptor::Float,
            ValueView::String(_) | ValueView::ShortString(_) => TypeDescriptor::String,
            ValueView::Vec(vector) => {
                let mut members = Vec::new();
                for element in vector.iter() {
                    push_runtime_union_member(
                        &mut members,
                        self.runtime_type_descriptor(element, depth + 1),
                    );
                }
                TypeDescriptor::Vector(Some(Box::new(runtime_union(members))))
            }
            ValueView::Dict(dictionary) => {
                let mut keys = Vec::new();
                let mut values = Vec::new();
                for (key, value) in dictionary.iter() {
                    push_runtime_union_member(
                        &mut keys,
                        match key {
                            KeyRef::Int(_) => TypeDescriptor::Int,
                            KeyRef::Bool(_) => TypeDescriptor::Bool,
                            KeyRef::String(_) | KeyRef::ShortString(_) => TypeDescriptor::String,
                        },
                    );
                    push_runtime_union_member(
                        &mut values,
                        self.runtime_type_descriptor(value, depth + 1),
                    );
                }
                TypeDescriptor::Dictionary(Some((
                    Box::new(runtime_union(keys)),
                    Box::new(runtime_union(values)),
                )))
            }
            ValueView::Tuple(tuple) => TypeDescriptor::Tuple(
                tuple
                    .iter()
                    .map(|element| self.runtime_type_descriptor(element, depth + 1))
                    .collect(),
            ),
            ValueView::Function(function) => TypeDescriptor::Callable(Some(
                self.callable_type_descriptor(function, function.type_environment()),
            )),
            ValueView::Object(instance) => {
                let class = &self.engine.tables.classes[instance.class().0 as usize];
                let arguments = (!class.type_parameters.is_empty()).then(|| {
                    class
                        .type_parameters
                        .iter()
                        .map(|parameter| {
                            self.type_environment_binding(
                                instance.type_environment(),
                                &parameter.name,
                            )
                            .cloned()
                            .unwrap_or(TypeDescriptor::Mixed)
                        })
                        .collect()
                });
                TypeDescriptor::Named {
                    name: class.name.clone(),
                    arguments,
                    recursive: false,
                }
            }
            ValueView::Iter(_) => TypeDescriptor::Object,
        }
    }

    /// The canonical runtime type spelling used by diagnostics and `dbg!`.
    pub(in crate::vm) fn runtime_type_name(&self, value: &Value) -> String {
        self.render_descriptor(&self.runtime_type_descriptor(value, 0))
    }

    /// Renders a type descriptor for diagnostics.
    pub(crate) fn render_descriptor(&self, descriptor: &TypeDescriptor) -> String {
        render::type_descriptor(descriptor, &|value| {
            String::from_utf8_lossy(ops::render_float(&self.heap, value).flatten()).into_owned()
        })
    }
}
