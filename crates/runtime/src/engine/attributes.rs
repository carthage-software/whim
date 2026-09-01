//! Validating attribute applications.

use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::ALWAYS_INLINE_ATTRIBUTE;
use crate::bytecode::unit::COLD_ATTRIBUTE;
use crate::bytecode::unit::CompiledAttribute;
use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::FRAMELESS_ATTRIBUTE;
use crate::bytecode::unit::MUST_USE_ATTRIBUTE;
use crate::bytecode::unit::NEVER_INLINE_ATTRIBUTE;
use crate::bytecode::unit::TRACK_CALLER_ATTRIBUTE;
use crate::bytecode::unit::frameless_literal;

use crate::bytecode::chunk::descriptors::check_trivial_descriptor;
use crate::bytecode::unit::literal_value;
use crate::engine::Atom;
use crate::engine::CompiledUnit;
use crate::engine::Engine;
use crate::engine::MethodBodyKind;
use crate::engine::Rc;
use crate::engine::SymbolKind;
use crate::engine::UnitContext;
use crate::engine::VirtualMachineControl;

const TARGET_ALL: i64 = 959;
const IS_REPEATABLE: i64 = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttributeTarget {
    Class,
    Function,
    Method,
    Property,
    ClassConstant,
    Parameter,
    TypeAlias,
    Newtype,
    Constant,
}

impl AttributeTarget {
    const fn flag(self) -> i64 {
        match self {
            Self::Class => 1,
            Self::Function => 2,
            Self::Method => 4,
            Self::Property => 8,
            Self::ClassConstant => 16,
            Self::Parameter => 32,
            Self::TypeAlias => 128,
            Self::Newtype => 256,
            Self::Constant => 512,
        }
    }

    const fn plural(self) -> &'static str {
        match self {
            Self::Class => "classes",
            Self::Function => "functions",
            Self::Method => "methods",
            Self::Property => "properties",
            Self::ClassConstant => "class constants or enum cases",
            Self::Parameter => "parameters",
            Self::TypeAlias => "type aliases",
            Self::Newtype => "newtypes",
            Self::Constant => "constants",
        }
    }
}

impl Engine {
    pub(crate) fn validate_unit_attributes(
        &mut self,
        unit: &Rc<CompiledUnit>,
    ) -> Result<(), VirtualMachineControl> {
        for class in &unit.classes {
            self.validate_class_attributes(class, &unit.path)?;
        }

        for function in &unit.functions {
            self.validate_function_attributes(function, &unit.path)?;
        }

        for alias in &unit.type_aliases {
            let where_ = alias.name.to_string_lossy().into_owned();
            self.validate_attribute_applications(
                &alias.attributes,
                &where_,
                AttributeTarget::TypeAlias,
                &unit.path,
            )?;
        }

        for newtype in &unit.newtypes {
            let where_ = newtype.name.to_string_lossy().into_owned();
            self.validate_attribute_applications(
                &newtype.attributes,
                &where_,
                AttributeTarget::Newtype,
                &unit.path,
            )?;
        }

        for constant in &unit.constants {
            let where_ = constant.name.to_string_lossy().into_owned();
            self.validate_attribute_applications(
                &constant.attributes,
                &where_,
                AttributeTarget::Constant,
                &unit.path,
            )?;
        }

        Ok(())
    }

    fn validate_class_attributes(
        &mut self,
        class: &CompiledClassLike,
        path: &Atom,
    ) -> Result<(), VirtualMachineControl> {
        let class_text = class.name.to_string_lossy();
        self.validate_attribute_applications(
            &class.attributes,
            &class_text,
            AttributeTarget::Class,
            path,
        )?;

        for constant in &class.constants {
            self.validate_attribute_applications(
                &constant.attributes,
                &format!("{class_text}::{}", constant.name),
                AttributeTarget::ClassConstant,
                path,
            )?;
        }

        for property in &class.properties {
            let declared = format!("{class_text}::${}", property.name);
            if property.is_promoted {
                self.validate_attribute_applications_with_alternative(
                    &property.attributes,
                    &declared,
                    AttributeTarget::Property,
                    AttributeTarget::Parameter,
                    path,
                )?;
            } else {
                self.validate_attribute_applications(
                    &property.attributes,
                    &declared,
                    AttributeTarget::Property,
                    path,
                )?;
            }
        }

        for case in &class.cases {
            self.validate_attribute_applications(
                &case.attributes,
                &format!("{class_text}::{}", case.name),
                AttributeTarget::ClassConstant,
                path,
            )?;
        }

        for method in &class.methods {
            let declared = format!("{class_text}::{}", method.name);
            let is_constructor = method.name == self.tables.constructor_name;
            self.validate_attribute_applications(
                &method.function.attributes,
                &declared,
                AttributeTarget::Method,
                path,
            )?;
            self.validate_callable_markers(&method.function, &declared, path)?;
            self.validate_frameless_body(&method.function, &declared, path, is_constructor)?;

            let promoted = if is_constructor {
                class
                    .properties
                    .iter()
                    .filter(|property| property.is_promoted)
                    .map(|property| &property.name)
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            self.validate_parameter_attributes(&method.function, &declared, &promoted, path)?;
        }

        Ok(())
    }

    fn validate_function_attributes(
        &mut self,
        function: &CompiledFunction,
        path: &Atom,
    ) -> Result<(), VirtualMachineControl> {
        if function.attributes.is_empty() && function.parameters.is_empty() {
            return Ok(());
        }

        let declared = function.name.to_string_lossy();
        self.validate_attribute_applications(
            &function.attributes,
            &declared,
            AttributeTarget::Function,
            path,
        )?;
        self.validate_callable_markers(function, &declared, path)?;
        self.validate_frameless_body(function, &declared, path, false)?;
        self.validate_parameter_attributes(function, &declared, &[], path)
    }

    fn validate_callable_markers(
        &mut self,
        function: &CompiledFunction,
        declared: &str,
        path: &Atom,
    ) -> Result<(), VirtualMachineControl> {
        if let Some(attribute) = function
            .attributes
            .iter()
            .find(|attribute| attribute.class.as_bytes() == MUST_USE_ATTRIBUTE)
            && matches!(
                function.return_type,
                Some(TypeDescriptor::Void | TypeDescriptor::Never)
            )
        {
            return Err(self.linker_error_at(
                path,
                attribute.span,
                format!(
                    "the attribute {} cannot be applied to {declared} because its result can never be used",
                    attribute.class
                ),
            ));
        }

        let always = function
            .attributes
            .iter()
            .find(|attribute| attribute.class.as_bytes() == ALWAYS_INLINE_ATTRIBUTE);
        let Some(always) = always else {
            return Ok(());
        };

        let conflict = function.attributes.iter().find(|attribute| {
            matches!(
                attribute.class.as_bytes(),
                NEVER_INLINE_ATTRIBUTE
                    | COLD_ATTRIBUTE
                    | TRACK_CALLER_ATTRIBUTE
                    | FRAMELESS_ATTRIBUTE
            )
        });
        let Some(conflict) = conflict else {
            return Ok(());
        };

        Err(self.linker_error_at(
            path,
            always.span,
            format!(
                "the attribute {} on {declared} conflicts with {}",
                always.class, conflict.class
            ),
        ))
    }

    fn validate_frameless_body(
        &mut self,
        function: &CompiledFunction,
        declared: &str,
        path: &Atom,
        is_constructor: bool,
    ) -> Result<(), VirtualMachineControl> {
        let Some(attribute) = function
            .attributes
            .iter()
            .find(|attribute| attribute.class.as_bytes() == FRAMELESS_ATTRIBUTE)
        else {
            return Ok(());
        };
        if !is_constructor
            && function.parameters.is_empty()
            && let Some(literal) = frameless_literal(function)
            && function.return_type.as_ref().is_none_or(|descriptor| {
                check_trivial_descriptor(descriptor, &literal_value(&literal)) == Some(true)
            })
        {
            return Ok(());
        }

        Err(self.linker_error_at(
            path,
            attribute.span,
            format!(
                "the attribute {} on {declared} requires a non-constructor with zero parameters and a body that returns exactly one literal satisfying its declared return type",
                attribute.class
            ),
        ))
    }

    fn validate_parameter_attributes(
        &mut self,
        function: &CompiledFunction,
        declared: &str,
        promoted: &[&Atom],
        path: &Atom,
    ) -> Result<(), VirtualMachineControl> {
        for parameter in &function.parameters {
            if parameter.attributes.is_empty() {
                continue;
            }

            let where_ = format!("{declared}(${})", parameter.name);
            if promoted.contains(&&parameter.name) {
                self.validate_attribute_applications_with_alternative(
                    &parameter.attributes,
                    &where_,
                    AttributeTarget::Parameter,
                    AttributeTarget::Property,
                    path,
                )?;
            } else {
                self.validate_attribute_applications(
                    &parameter.attributes,
                    &where_,
                    AttributeTarget::Parameter,
                    path,
                )?;
            }
        }

        Ok(())
    }

    pub(crate) fn extract_attribute_flags(
        &mut self,
        attributes: &[CompiledAttribute],
        context: &Rc<UnitContext>,
        path: &Atom,
    ) -> Result<Option<i64>, VirtualMachineControl> {
        let marker = self.heap.intern(b"Whim\\Attribute\\Attribute");
        for attribute in attributes {
            if attribute.class != marker {
                continue;
            }

            let initializer = attribute.arguments.first().or_else(|| {
                attribute
                    .named_arguments
                    .iter()
                    .find(|(name, _)| name.as_bytes() == b"flags")
                    .map(|(_, initializer)| initializer)
            });

            let Some(initializer) = initializer else {
                return Ok(Some(TARGET_ALL));
            };

            let value = self.evaluate_initializer(initializer, context)?;
            let Some(flags) = value.as_int() else {
                return Err(self.linker_error_at(
                    path,
                    attribute.span,
                    format!(
                        "the Whim\\Attribute\\Attribute flags must be int, {} given",
                        value.kind_name()
                    ),
                ));
            };

            return Ok(Some(flags));
        }

        Ok(None)
    }

    fn validate_attribute_applications(
        &mut self,
        attributes: &[CompiledAttribute],
        declared: &str,
        target: AttributeTarget,
        path: &Atom,
    ) -> Result<(), VirtualMachineControl> {
        self.validate_attribute_applications_for(attributes, declared, target, None, path)
    }

    fn validate_attribute_applications_with_alternative(
        &mut self,
        attributes: &[CompiledAttribute],
        declared: &str,
        target: AttributeTarget,
        alternative: AttributeTarget,
        path: &Atom,
    ) -> Result<(), VirtualMachineControl> {
        self.validate_attribute_applications_for(
            attributes,
            declared,
            target,
            Some(alternative),
            path,
        )
    }

    fn validate_attribute_applications_for(
        &mut self,
        attributes: &[CompiledAttribute],
        declared: &str,
        target: AttributeTarget,
        alternative: Option<AttributeTarget>,
        path: &Atom,
    ) -> Result<(), VirtualMachineControl> {
        for (position, attribute) in attributes.iter().enumerate() {
            let attribute_text = attribute.class.to_string();
            let Some(entry) = self.tables.symbols.get(&attribute.class).copied() else {
                return Err(self.linker_error_at(
                    path,
                    attribute.span,
                    format!("the attribute class {attribute_text} is not defined"),
                ));
            };

            if entry.kind != SymbolKind::Class {
                return Err(self.linker_error_at(
                    path,
                    attribute.span,
                    format!("{attribute_text} is not a class and cannot be an attribute"),
                ));
            }

            let applied = &self.tables.classes[entry.index as usize];
            let Some(flags) = applied.attribute_flags else {
                return Err(self.linker_error_at(
                    path,
                    attribute.span,
                    format!(
                        "{attribute_text} does not carry #[Whim\\Attribute\\Attribute] and cannot be applied to {declared}"
                    ),
                ));
            };

            if flags & target.flag() == 0
                && alternative.is_none_or(|alternative| flags & alternative.flag() == 0)
            {
                let targets = alternative.map_or_else(
                    || target.plural().to_string(),
                    |alternative| format!("{} or {}", target.plural(), alternative.plural()),
                );
                return Err(self.linker_error_at(
                    path,
                    attribute.span,
                    format!(
                        "the attribute {attribute_text} does not target {targets}; it is applied \
                         to {declared}"
                    ),
                ));
            }

            self.check_attribute_arity(attribute, entry.index, &attribute_text, declared, path)?;
            let repeated = attributes[..position]
                .iter()
                .any(|earlier| earlier.class == attribute.class);
            if repeated && flags & IS_REPEATABLE == 0 {
                return Err(self.linker_error_at(
                    path,
                    attribute.span,
                    format!("the attribute {attribute_text} is not repeatable"),
                ));
            }
        }

        Ok(())
    }

    /// Checks an attribute's arguments against its class's constructor, so a
    /// declaration that could never be constructed is rejected where it is
    /// written rather than when something reflects on it.
    fn check_attribute_arity(
        &mut self,
        attribute: &CompiledAttribute,
        attribute_class: u32,
        attribute_text: &str,
        declared: &str,
        path: &Atom,
    ) -> Result<(), VirtualMachineControl> {
        let constructor =
            self.tables.classes[attribute_class as usize].method(&self.tables.constructor_name);

        let supplied = attribute.arguments.len();
        let Some(entry) = constructor else {
            if supplied == 0 && attribute.named_arguments.is_empty() {
                return Ok(());
            }

            return Err(self.linker_error_at(
                path,
                attribute.span,
                format!(
                    "the attribute {attribute_text} on {declared} is given arguments, and \
                     {attribute_text} declares no constructor"
                ),
            ));
        };

        let MethodBodyKind::Bytecode(function) = entry.body else {
            return Ok(());
        };

        let function = &self.tables.functions[function.0 as usize];
        let required = usize::from(function.required_parameters);
        let declared_count = usize::from(function.declared_parameters);
        let named = attribute.named_arguments.len();
        let total = supplied + named;
        if total < required || total > declared_count {
            let expected = if required == declared_count {
                format!("exactly {required}")
            } else {
                format!("{required} to {declared_count}")
            };

            return Err(self.linker_error_at(
                path,
                attribute.span,
                format!(
                    "the attribute {attribute_text} on {declared} is given {total} argument(s), \
                     and its constructor takes {expected}"
                ),
            ));
        }

        Ok(())
    }
}
