//! Building attribute instances, lazily and once.

use whim_span::Span;

use crate::bytecode::unit::BuiltInCallableAttributes;
use crate::bytecode::unit::CompiledAttribute;
use crate::bytecode::unit::ConstantInitializer;
use crate::bytecode::unit::MUST_USE_ATTRIBUTE;
use crate::bytecode::unit::TRACE_BOUNDARY_ATTRIBUTE;
use crate::bytecode::unit::TRACK_CALLER_ATTRIBUTE;
use crate::bytecode::unit::literal_value;
use crate::core::symbols::strip_leading_backslash;
use crate::symbols::FunctionTable;
use crate::symbols::SymbolKind;
use crate::vm::Atom;
use crate::vm::CallTarget;
use crate::vm::CalleeShape;
use crate::vm::ClassId;
use crate::vm::FunctionLocator;
use crate::vm::MethodBodyKind;
use crate::vm::Rc;
use crate::vm::Throw;
use crate::vm::UnitContext;
use crate::vm::Value;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;
use crate::vm::unreachable_invariant;

#[derive(Clone, Copy)]
pub(in crate::vm) enum MemberKind {
    Property,
    Constant,
    EnumCase,
}

enum ParameterSelector<'a> {
    Position(usize),
    Name(&'a Atom),
}

impl VirtualMachine<'_> {
    fn built_in_callable_attributes(
        &self,
        attributes: BuiltInCallableAttributes,
    ) -> Vec<CompiledAttribute> {
        let mut declarations = Vec::with_capacity(
            usize::from(attributes.track_caller)
                + usize::from(attributes.trace_boundary)
                + usize::from(attributes.must_use),
        );
        if attributes.track_caller {
            declarations.push(CompiledAttribute {
                class: self.heap.intern(TRACK_CALLER_ATTRIBUTE),
                span: Span::zero(),
                arguments: Vec::new(),
                named_arguments: Vec::new(),
            });
        }
        if attributes.trace_boundary {
            declarations.push(CompiledAttribute {
                class: self.heap.intern(TRACE_BOUNDARY_ATTRIBUTE),
                span: Span::zero(),
                arguments: Vec::new(),
                named_arguments: Vec::new(),
            });
        }
        if attributes.must_use {
            declarations.push(CompiledAttribute {
                class: self.heap.intern(MUST_USE_ATTRIBUTE),
                span: Span::zero(),
                arguments: Vec::new(),
                named_arguments: Vec::new(),
            });
        }

        declarations
    }

    /// The attribute instances of a class, built on first inspection and
    /// cached. The constructor arguments are constant expressions, so
    /// construction cannot observe program state.
    pub(crate) fn attribute_instances_of(
        &mut self,
        class: ClassId,
    ) -> Result<Vec<Value>, VirtualMachineControl> {
        let declarations = self.engine.tables.classes[class.0 as usize]
            .attribute_declarations
            .clone();
        let unit = self.engine.tables.classes[class.0 as usize]
            .attribute_unit
            .clone();
        self.build_attribute_instances(&declarations, unit.as_ref())
    }

    pub(crate) fn build_attribute_instances(
        &mut self,
        declarations: &[CompiledAttribute],
        unit: Option<&Rc<UnitContext>>,
    ) -> Result<Vec<Value>, VirtualMachineControl> {
        let mut instances = Vec::with_capacity(declarations.len());
        for declaration in declarations {
            let Some(entry) = self.engine.tables.symbols.get(&declaration.class).copied() else {
                // SAFETY: the surrounding invariant makes this path unreachable.
                unsafe { unreachable_invariant("attribute classes were validated at link") }
            };

            let attribute_class = ClassId(entry.index);
            let mut positional = Vec::with_capacity(declaration.arguments.len());
            for initializer in &declaration.arguments {
                positional.push(self.constant_argument(initializer, unit)?);
            }

            let mut named = Vec::with_capacity(declaration.named_arguments.len());
            for (name, initializer) in &declaration.named_arguments {
                named.push((name.clone(), self.constant_argument(initializer, unit)?));
            }

            let arguments = if named.is_empty() {
                positional
            } else {
                self.attribute_arguments_with_named(attribute_class, positional, named)?
            };

            instances.push(self.instantiate_class(attribute_class, &arguments)?);
        }

        Ok(instances)
    }

    /// Evaluates one attribute argument. The compiler admits only constant
    /// expressions here, so this cannot reach program state.
    fn constant_argument(
        &mut self,
        initializer: &ConstantInitializer,
        unit: Option<&Rc<UnitContext>>,
    ) -> Result<Value, VirtualMachineControl> {
        match unit {
            Some(unit) => self.engine.evaluate_initializer(initializer, unit),
            None => match initializer {
                ConstantInitializer::Literal(literal) => Ok(literal_value(literal)),
                // SAFETY: the surrounding invariant makes this path unreachable.
                ConstantInitializer::Thunk(_) => unsafe {
                    unreachable_invariant("engine classes carry no argument thunks")
                },
            },
        }
    }

    fn attribute_arguments_with_named(
        &mut self,
        attribute_class: ClassId,
        positional: Vec<Value>,
        named: Vec<(Atom, Value)>,
    ) -> Result<Vec<Value>, VirtualMachineControl> {
        let constructor = self.engine.tables.classes[attribute_class.0 as usize]
            .method(&self.engine.tables.constructor_name.clone());

        let Some(entry) = constructor else {
            return Err(self.throw_well_known(
                self.engine.tables.well_known.argument_count_error,
                "named arguments require a constructor".to_string(),
            ));
        };

        let target = match entry.body {
            MethodBodyKind::Bytecode(function) => CallTarget::User(function),
            MethodBodyKind::BuiltIn(_) => CallTarget::BuiltIn(
                self.built_in_id_for_method(&entry, self.engine.tables.constructor_name.clone()),
            ),
        };

        let shape = CalleeShape {
            target,
            this: None,
            holder: None,
            method: None,
        };

        self.build_final_arguments(&shape, positional, &named)
    }
}

impl VirtualMachine<'_> {
    /// Whether the class carries an attribute declaration naming `attribute`.
    /// Never re-enters the interpreter.
    pub(crate) fn class_has_attribute(&self, class: ClassId, attribute: Atom) -> bool {
        let wanted = strip_leading_backslash(&self.heap, attribute);
        self.engine.tables.classes[class.0 as usize]
            .attribute_declarations
            .iter()
            .any(|declaration| declaration.class == wanted)
    }

    pub(crate) fn class_attributes(&mut self, class: ClassId) -> Result<Vec<Value>, Throw> {
        self.attribute_instances_of(class)
            .map_err(|control| self.control_to_throw(control))
    }

    pub(crate) fn function_attributes(&mut self, function: Atom) -> Result<Vec<Value>, Throw> {
        let wanted = strip_leading_backslash(&self.heap, function);
        let Some(entry) = self.engine.tables.symbols.get(&wanted).copied() else {
            return Ok(Vec::new());
        };

        if entry.kind != SymbolKind::Function {
            return Ok(Vec::new());
        }

        let (declarations, unit) = match entry.table {
            FunctionTable::User => {
                let runtime = &self.engine.tables.functions[entry.index as usize];
                let unit = Rc::clone(&runtime.unit);
                let FunctionLocator::TopLevel(index) = runtime.locator else {
                    return Ok(Vec::new());
                };
                (
                    unit.unit.functions[index as usize].attributes.clone(),
                    Some(unit),
                )
            }
            FunctionTable::BuiltIn => (
                self.built_in_callable_attributes(
                    self.engine.tables.built_in_function_declarations[entry.index as usize]
                        .attributes,
                ),
                None,
            ),
        };
        self.build_attribute_instances(&declarations, unit.as_ref())
            .map_err(|control| self.control_to_throw(control))
    }

    pub(crate) fn method_attributes(
        &mut self,
        class: ClassId,
        method: Atom,
    ) -> Result<Vec<Value>, Throw> {
        let Some(entry) = self.engine.tables.classes[class.0 as usize].method(&method) else {
            return Ok(Vec::new());
        };

        let (declarations, unit) = match entry.body {
            MethodBodyKind::Bytecode(function) => {
                let runtime = &self.engine.tables.functions[function.0 as usize];
                let unit = Rc::clone(&runtime.unit);
                let FunctionLocator::Method {
                    class: class_index,
                    method: method_index,
                } = runtime.locator
                else {
                    return Ok(Vec::new());
                };
                (
                    unit.unit.classes[class_index as usize].methods[method_index as usize]
                        .function
                        .attributes
                        .clone(),
                    Some(unit),
                )
            }
            MethodBodyKind::BuiltIn(body) => {
                (self.built_in_callable_attributes(body.attributes), None)
            }
        };

        self.build_attribute_instances(&declarations, unit.as_ref())
            .map_err(|control| self.control_to_throw(control))
    }

    pub(crate) fn parameter_attributes(
        &mut self,
        class: ClassId,
        method: Atom,
        position: usize,
    ) -> Result<Vec<Value>, Throw> {
        self.parameter_attributes_by(class, method, ParameterSelector::Position(position))
    }

    pub(crate) fn parameter_attributes_named(
        &mut self,
        class: ClassId,
        method: Atom,
        parameter: Atom,
    ) -> Result<Vec<Value>, Throw> {
        self.parameter_attributes_by(class, method, ParameterSelector::Name(&parameter))
    }

    fn parameter_attributes_by(
        &mut self,
        class: ClassId,
        method: Atom,
        selector: ParameterSelector<'_>,
    ) -> Result<Vec<Value>, Throw> {
        let Some(entry) = self.engine.tables.classes[class.0 as usize].method(&method) else {
            return Ok(Vec::new());
        };

        let MethodBodyKind::Bytecode(function) = entry.body else {
            return Ok(Vec::new());
        };

        let runtime = &self.engine.tables.functions[function.0 as usize];
        let unit = Rc::clone(&runtime.unit);
        let FunctionLocator::Method {
            class: class_index,
            method: method_index,
        } = runtime.locator
        else {
            return Ok(Vec::new());
        };

        let compiled = &unit.unit.classes[class_index as usize].methods[method_index as usize];
        let parameter = match selector {
            ParameterSelector::Position(position) => compiled.function.parameters.get(position),
            ParameterSelector::Name(name) => compiled
                .function
                .parameters
                .iter()
                .find(|parameter| parameter.name == *name),
        };

        let Some(parameter) = parameter else {
            return Ok(Vec::new());
        };

        let declarations = parameter.attributes.clone();
        self.build_attribute_instances(&declarations, Some(&unit))
            .map_err(|control| self.control_to_throw(control))
    }

    pub(crate) fn property_attributes(
        &mut self,
        class: ClassId,
        property: Atom,
    ) -> Result<Vec<Value>, Throw> {
        self.member_attributes(class, property, MemberKind::Property)
    }

    pub(crate) fn class_constant_attributes(
        &mut self,
        class: ClassId,
        constant: Atom,
    ) -> Result<Vec<Value>, Throw> {
        self.member_attributes(class, constant, MemberKind::Constant)
    }

    pub(crate) fn enum_case_attributes(
        &mut self,
        class: ClassId,
        case: Atom,
    ) -> Result<Vec<Value>, Throw> {
        self.member_attributes(class, case, MemberKind::EnumCase)
    }

    /// The attributes of a property, constant, or enum case, found through
    /// the class's own declaration in its unit.
    fn member_attributes(
        &mut self,
        class: ClassId,
        member: Atom,
        kind: MemberKind,
    ) -> Result<Vec<Value>, Throw> {
        let Some(unit) = self.engine.tables.classes[class.0 as usize]
            .attribute_unit
            .clone()
        else {
            return Ok(Vec::new());
        };

        let name = self.engine.tables.classes[class.0 as usize].name.clone();
        let Some(compiled) = unit.unit.classes.iter().find(|entry| entry.name == name) else {
            return Ok(Vec::new());
        };

        let declarations = match kind {
            MemberKind::Property => compiled
                .properties
                .iter()
                .find(|property| property.name == member)
                .map(|property| property.attributes.clone()),
            MemberKind::Constant => compiled
                .constants
                .iter()
                .find(|constant| constant.name == member)
                .map(|constant| constant.attributes.clone()),
            MemberKind::EnumCase => compiled
                .cases
                .iter()
                .find(|case| case.name == member)
                .map(|case| case.attributes.clone()),
        };

        let Some(declarations) = declarations else {
            return Ok(Vec::new());
        };

        self.build_attribute_instances(&declarations, Some(&unit))
            .map_err(|control| self.control_to_throw(control))
    }

    pub(crate) fn class_attribute(
        &mut self,
        class: ClassId,
        attribute: Atom,
    ) -> Result<Vec<Value>, Throw> {
        let wanted = strip_leading_backslash(&self.heap, attribute);
        let wanted_class = self
            .engine
            .tables
            .symbols
            .get(&wanted)
            .map(|entry| entry.index);
        let instances = self.class_attributes(class)?;
        Ok(instances
            .into_iter()
            .filter(|attribute| {
                attribute
                    .as_object()
                    .is_some_and(|built| Some(built.class().0) == wanted_class)
            })
            .collect())
    }
}
