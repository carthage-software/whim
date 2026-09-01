//! Building attribute instances.

use crate::bytecode::unit::CompiledAttribute;
use crate::bytecode::unit::ConstantInitializer;
use crate::bytecode::unit::literal_value;
use crate::vm::Atom;
use crate::vm::CallTarget;
use crate::vm::CalleeShape;
use crate::vm::ClassId;
use crate::vm::MethodBodyKind;
use crate::vm::Rc;
use crate::vm::UnitContext;
use crate::vm::Value;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;
use crate::vm::unreachable_invariant;

impl VirtualMachine<'_> {
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
