//! Shared native state and method dispatch for reflection objects.

use std::cell::RefCell;

use crate::builtin::Context;
use crate::builtin::arguments::Arguments;
use crate::builtin::throw::Throw;
use crate::core::reflection::Operation;
use crate::core::reflection::attributes;
use crate::core::reflection::classes;
use crate::core::reflection::declarations;
use crate::core::reflection::metadata;
use crate::core::reflection::model::DeclarationKey;
use crate::core::reflection::model::ReflectionData;
use crate::core::reflection::types;
use crate::core::reflection::values;
use crate::value::Value;
use crate::value::heap::metadata::TeardownMode;
use crate::value::heap::metadata::TraceVisitor;
use crate::value::heap::queue::DropQueue;

#[derive(Default)]
pub(crate) struct ReflectionState {
    pub(crate) data: RefCell<Option<ReflectionData>>,
    pub(crate) values: RefCell<Vec<Value>>,
}

impl ReflectionState {
    pub(crate) fn initialize(&self, data: ReflectionData, values: Vec<Value>) {
        *self.data.borrow_mut() = Some(data);
        *self.values.borrow_mut() = values;
    }

    fn snapshot(&self) -> Option<(ReflectionData, Vec<Value>)> {
        Some((
            self.data.borrow().as_ref()?.clone(),
            self.values.borrow().clone(),
        ))
    }

    pub(crate) fn enqueue_children(&mut self, queue: &DropQueue, mode: TeardownMode) {
        for value in self.values.get_mut().drain(..) {
            queue.release_value(value, mode);
        }
    }

    pub(crate) fn visit_children(&self, visitor: &mut TraceVisitor<'_>) {
        for value in self.values.borrow().iter() {
            if let Some(child) = value.collectable_box() {
                visitor.visit(child);
            }
        }
    }
}

pub(crate) fn dispatch(
    context: &mut Context<'_, '_, '_>,
    arguments: Arguments<'_>,
    operation: Operation,
) -> Result<Value, Throw> {
    let receiver = context.receiver();
    let Some(state) = classes::state(&receiver) else {
        return Err(context.type_error("the reflection object has no built-in state"));
    };
    let Some((data, values)) = state.snapshot() else {
        return Err(context.type_error("the reflection object is not initialized"));
    };
    match data {
        ReflectionData::SourceLocation(location) => {
            metadata::source_location_dispatch(context, operation, &location)
        }
        ReflectionData::Symbol(name) => {
            let declaration = DeclarationKey::Symbol(name.clone());
            if let Some(result) =
                metadata::declaration_dispatch(context, arguments, operation, &declaration)
            {
                return result;
            }
            declarations::symbol_dispatch(context, arguments, operation, &name)
        }
        ReflectionData::Member(member) => {
            let declaration = DeclarationKey::Member(member.clone());
            if let Some(result) =
                metadata::declaration_dispatch(context, arguments, operation, &declaration)
            {
                return result;
            }
            declarations::member_dispatch(context, arguments, operation, &member)
        }
        ReflectionData::Parameter { callable, position } => {
            let declaration = DeclarationKey::Parameter {
                callable: callable.clone(),
                position,
            };
            if let Some(result) =
                metadata::declaration_dispatch(context, arguments, operation, &declaration)
            {
                return result;
            }
            declarations::parameter_dispatch(context, arguments, operation, &callable, position)
        }
        ReflectionData::Closure(function) => {
            let declaration = DeclarationKey::Closure(function);
            if let Some(result) =
                metadata::declaration_dispatch(context, arguments, operation, &declaration)
            {
                return result;
            }
            declarations::closure_dispatch(context, arguments, operation, function)
        }
        ReflectionData::Capture { function, position } => {
            declarations::capture_dispatch(context, operation, function, position)
        }
        ReflectionData::TypeParameter(parameter) => {
            declarations::type_parameter_dispatch(context, operation, &parameter)
        }
        ReflectionData::Attribute {
            target,
            declaration,
            unit,
        } => {
            attributes::attribute_dispatch(context, operation, &target, &declaration, unit.as_ref())
        }
        ReflectionData::AttributeDefinition(class) => {
            attributes::definition_dispatch(context, operation, class)
        }
        ReflectionData::Type(reflected) => {
            types::dispatch(context, arguments, operation, &reflected)
        }
        ReflectionData::FunctionTypeParameter {
            position,
            r#type,
            optional,
        } => types::function_parameter_dispatch(context, operation, position, &r#type, optional),
        ReflectionData::DictShapeEntry { key, r#type } => {
            types::shape_entry_dispatch(context, operation, &key, &r#type)
        }
        ReflectionData::TypeEnvironment(bindings) => {
            types::environment_dispatch(context, arguments, operation, &bindings)
        }
        ReflectionData::TypeBinding {
            parameter,
            argument,
        } => types::binding_dispatch(context, operation, &parameter, &argument),
        ReflectionData::Object => values::object_dispatch(context, arguments, operation, &values),
        ReflectionData::PropertyValue { property, slot } => {
            values::property_value_dispatch(context, operation, &property, slot, &values)
        }
        ReflectionData::CallableValue => values::callable_dispatch(context, operation, &values),
        ReflectionData::CaptureValue { function, position } => {
            values::capture_value_dispatch(context, operation, function, position, &values)
        }
        ReflectionData::BoundArgument {
            callable,
            parameter,
        } => values::bound_argument_dispatch(context, operation, &callable, parameter, &values),
        ReflectionData::NewtypeValue(identifier) => {
            values::newtype_value_dispatch(context, operation, identifier, &values)
        }
    }
}
