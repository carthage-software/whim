//! Validation of source declarations supplied by an already loaded provider.

use std::rc::Rc;

use hashbrown::HashMap;

use whim_span::Span;

use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::ClassLikeKind;
use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::CompiledParameter;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::unit::Visibility;
use crate::bytecode::unit::is_external;
use crate::classes::MethodBodyKind;
use crate::classes::RuntimeBase;
use crate::engine::Engine;
use crate::engine::builtins::BuiltInCallable;
use crate::engine::builtins::built_in_parameters;
use crate::engine::builtins::built_in_type_parameters;
use crate::linker::descriptors::descriptor_from_built_in_spec;
use crate::symbols::FunctionTable;
use crate::symbols::SymbolEntry;
use crate::symbols::SymbolKind;
use crate::symbols::UnitContext;
use crate::unreachable_invariant;
use crate::value::atom::Atom;
use crate::value::object::ClassId;
use crate::vm::VirtualMachineControl;

struct FunctionShape {
    type_parameters: Vec<CompiledTypeParameter>,
    parameters: Vec<CompiledParameter>,
    return_type: Option<TypeDescriptor>,
}

struct MethodShape {
    visibility: Visibility,
    is_static: bool,
    is_abstract: bool,
    is_final: bool,
    function: FunctionShape,
}

struct PropertyShape {
    visibility: Visibility,
    is_static: bool,
    is_readonly: bool,
    declared_type: Option<TypeDescriptor>,
}

struct ConstantShape {
    visibility: Visibility,
    declared_type: Option<TypeDescriptor>,
}

struct ClassShape {
    kind: ClassLikeKind,
    is_abstract: bool,
    is_final: bool,
    is_readonly: bool,
    type_parameters: Vec<CompiledTypeParameter>,
    direct_bases: Vec<RuntimeBase>,
    constants: HashMap<Atom, ConstantShape>,
    properties: HashMap<Atom, PropertyShape>,
    methods: HashMap<Atom, MethodShape>,
    cases: Vec<Atom>,
    sealed_to: Option<Vec<Atom>>,
    attribute_flags: Option<i64>,
}

impl Engine {
    pub(crate) fn validate_external_declarations(
        &mut self,
        unit: &CompiledUnit,
        context: &Rc<UnitContext>,
    ) -> Result<(), VirtualMachineControl> {
        for function in &unit.functions {
            if is_external(&function.attributes) {
                self.validate_external_function(function, unit)?;
            }
        }

        for class in &unit.classes {
            if is_external(&class.attributes) {
                self.validate_external_class(class, unit, context)?;
            }
        }

        for constant in &unit.constants {
            if is_external(&constant.attributes) {
                self.require_external_kind(
                    &constant.name,
                    SymbolKind::Constant,
                    constant.span,
                    unit,
                )?;
            }
        }

        for alias in &unit.type_aliases {
            if !is_external(&alias.attributes) {
                continue;
            }

            let entry =
                self.require_external_kind(&alias.name, SymbolKind::TypeAlias, alias.span, unit)?;
            let actual = self.tables.type_aliases[entry.index as usize]
                .type_parameters
                .clone();
            self.check_type_parameters(
                &alias.type_parameters,
                &actual,
                &alias.name,
                alias.span,
                unit,
            )?;
        }

        for newtype in &unit.newtypes {
            if !is_external(&newtype.attributes) {
                continue;
            }

            let entry =
                self.require_external_kind(&newtype.name, SymbolKind::Newtype, newtype.span, unit)?;
            let actual = self.tables.newtypes[entry.index as usize]
                .type_parameters
                .clone();
            self.check_type_parameters(
                &newtype.type_parameters,
                &actual,
                &newtype.name,
                newtype.span,
                unit,
            )?;
        }

        Ok(())
    }

    fn validate_external_function(
        &mut self,
        expected: &CompiledFunction,
        unit: &CompiledUnit,
    ) -> Result<(), VirtualMachineControl> {
        let entry =
            self.require_external_kind(&expected.name, SymbolKind::Function, expected.span, unit)?;
        let actual = self.function_shape(entry);
        self.check_function_shape(expected, &actual, &expected.name, expected.span, unit)
    }

    fn validate_external_class(
        &mut self,
        expected: &CompiledClassLike,
        unit: &CompiledUnit,
        context: &Rc<UnitContext>,
    ) -> Result<(), VirtualMachineControl> {
        let expected_kind = symbol_kind(expected.kind);
        let entry =
            self.require_external_kind(&expected.name, expected_kind, expected.span, unit)?;
        let id = ClassId(entry.index);
        let actual = self.class_shape(id);

        self.check_external_class_header(expected, &actual, unit, context)?;
        self.check_external_constants(expected, &actual, unit)?;
        self.check_external_properties(expected, &actual, unit)?;
        self.check_external_methods(expected, &actual, unit)?;
        self.check_external_cases(expected, &actual, unit)
    }

    fn check_external_class_header(
        &mut self,
        expected: &CompiledClassLike,
        actual: &ClassShape,
        unit: &CompiledUnit,
        context: &Rc<UnitContext>,
    ) -> Result<(), VirtualMachineControl> {
        if actual.kind != expected.kind
            || actual.is_abstract
                != (expected.is_abstract
                    || matches!(
                        expected.kind,
                        ClassLikeKind::Interface | ClassLikeKind::Enum
                    ))
            || actual.is_final != expected.is_final
            || actual.is_readonly != expected.is_readonly
        {
            return Err(self.external_mismatch(
                &expected.name,
                expected.span,
                unit,
                "class-like modifiers do not match",
            ));
        }

        self.check_type_parameters(
            &expected.type_parameters,
            &actual.type_parameters,
            &expected.name,
            expected.span,
            unit,
        )?;

        self.check_bases(expected, &actual.direct_bases, unit)?;
        let permissions_match = if expected.kind == ClassLikeKind::Interface {
            match (&expected.sealed_to, &actual.sealed_to) {
                (None, _) => true,
                (Some(required), Some(registered)) => {
                    required.iter().all(|name| registered.contains(name))
                }
                (Some(_), None) => false,
            }
        } else {
            expected.sealed_to == actual.sealed_to
        };

        if !permissions_match {
            return Err(self.external_mismatch(
                &expected.name,
                expected.span,
                unit,
                "sealed permissions do not match",
            ));
        }

        let expected_flags =
            self.extract_attribute_flags(&expected.attributes, context, &unit.path)?;
        if expected_flags != actual.attribute_flags {
            return Err(self.external_mismatch(
                &expected.name,
                expected.span,
                unit,
                "attribute target flags do not match",
            ));
        }

        Ok(())
    }

    fn check_external_constants(
        &mut self,
        expected: &CompiledClassLike,
        actual: &ClassShape,
        unit: &CompiledUnit,
    ) -> Result<(), VirtualMachineControl> {
        if expected.constants.len() != actual.constants.len() {
            return Err(self.external_mismatch(
                &expected.name,
                expected.span,
                unit,
                "class constant set does not match",
            ));
        }

        for constant in &expected.constants {
            let Some(target) = actual.constants.get(&constant.name) else {
                return Err(self.external_mismatch(
                    &expected.name,
                    expected.span,
                    unit,
                    &format!("class constant {} is missing", constant.name),
                ));
            };

            if target.visibility != constant.visibility {
                return Err(self.external_mismatch(
                    &expected.name,
                    expected.span,
                    unit,
                    &format!("class constant {} has different visibility", constant.name),
                ));
            }

            if !self.optional_descriptors_equivalent(
                constant.declared_type.as_ref(),
                target.declared_type.as_ref(),
            )? {
                return Err(self.external_mismatch(
                    &expected.name,
                    expected.span,
                    unit,
                    &format!("class constant {} has a different type", constant.name),
                ));
            }
        }

        Ok(())
    }

    fn check_external_properties(
        &mut self,
        expected: &CompiledClassLike,
        actual: &ClassShape,
        unit: &CompiledUnit,
    ) -> Result<(), VirtualMachineControl> {
        if expected.properties.len() != actual.properties.len() {
            return Err(self.external_mismatch(
                &expected.name,
                expected.span,
                unit,
                "property set does not match",
            ));
        }

        for property in &expected.properties {
            let Some(target) = actual.properties.get(&property.name) else {
                return Err(self.external_mismatch(
                    &expected.name,
                    expected.span,
                    unit,
                    &format!("property ${} is missing", property.name),
                ));
            };

            if target.visibility != property.visibility
                || target.is_static != property.is_static
                || target.is_readonly != property.is_readonly
            {
                return Err(self.external_mismatch(
                    &expected.name,
                    expected.span,
                    unit,
                    &format!("property ${} has different modifiers", property.name),
                ));
            }

            if !self.optional_descriptors_equivalent(
                property.declared_type.as_ref(),
                target.declared_type.as_ref(),
            )? {
                return Err(self.external_mismatch(
                    &expected.name,
                    expected.span,
                    unit,
                    &format!("property ${} has a different type", property.name),
                ));
            }
        }

        Ok(())
    }

    fn check_external_methods(
        &mut self,
        expected: &CompiledClassLike,
        actual: &ClassShape,
        unit: &CompiledUnit,
    ) -> Result<(), VirtualMachineControl> {
        if expected.methods.len() != actual.methods.len() {
            return Err(self.external_mismatch(
                &expected.name,
                expected.span,
                unit,
                "method set does not match",
            ));
        }

        for method in &expected.methods {
            let Some(target) = actual.methods.get(&method.name) else {
                return Err(self.external_mismatch(
                    &expected.name,
                    expected.span,
                    unit,
                    &format!("method {} is missing", method.name),
                ));
            };

            if target.visibility != method.visibility
                || target.is_static != method.is_static
                || (expected.kind != ClassLikeKind::Interface
                    && target.is_abstract != method.is_abstract)
                || target.is_final != method.is_final
            {
                return Err(self.external_mismatch(
                    &expected.name,
                    expected.span,
                    unit,
                    &format!("method {} has different modifiers", method.name),
                ));
            }

            self.check_function_shape(
                &method.function,
                &target.function,
                &method.function.name,
                method.function.span,
                unit,
            )?;
        }

        Ok(())
    }

    fn check_external_cases(
        &mut self,
        expected: &CompiledClassLike,
        actual: &ClassShape,
        unit: &CompiledUnit,
    ) -> Result<(), VirtualMachineControl> {
        let expected_cases = expected
            .cases
            .iter()
            .map(|case| case.name.clone())
            .collect::<Vec<_>>();
        if expected_cases != actual.cases {
            return Err(self.external_mismatch(
                &expected.name,
                expected.span,
                unit,
                "enum cases do not match",
            ));
        }

        Ok(())
    }

    fn require_external_kind(
        &mut self,
        name: &Atom,
        expected: SymbolKind,
        span: Span,
        unit: &CompiledUnit,
    ) -> Result<SymbolEntry, VirtualMachineControl> {
        let Some(entry) = self.tables.symbols.get(name).copied() else {
            return Err(self.external_mismatch(name, span, unit, "the symbol is not registered"));
        };

        if entry.kind != expected {
            return Err(self.external_mismatch(
                name,
                span,
                unit,
                "the registered symbol has a different kind",
            ));
        }

        Ok(entry)
    }

    fn function_shape(&self, entry: SymbolEntry) -> FunctionShape {
        match entry.table {
            FunctionTable::User => {
                let function = &self.tables.functions[entry.index as usize];
                FunctionShape {
                    type_parameters: function.type_parameters().to_vec(),
                    parameters: function.parameters().to_vec(),
                    return_type: function.return_type.as_deref().cloned(),
                }
            }
            FunctionTable::BuiltIn => {
                let function = match &self.tables.built_in_functions[entry.index as usize] {
                    BuiltInCallable::Function(function) => function,
                    // SAFETY: function symbols only index built-in functions.
                    BuiltInCallable::Method { .. } => unsafe {
                        unreachable_invariant(
                            "a function symbol cannot reference a built-in method",
                        )
                    },
                };
                FunctionShape {
                    type_parameters: built_in_type_parameters(&self.heap, function.type_parameters),
                    parameters: built_in_parameters(&self.heap, function.parameters),
                    return_type: Some(descriptor_from_built_in_spec(
                        &self.heap,
                        &function.return_spec,
                    )),
                }
            }
        }
    }

    fn external_method_shape(&self, body: MethodBodyKind) -> FunctionShape {
        match body {
            MethodBodyKind::Bytecode(id) => {
                let function = &self.tables.functions[id.0 as usize];
                FunctionShape {
                    type_parameters: function.type_parameters().to_vec(),
                    parameters: function.parameters().to_vec(),
                    return_type: function.return_type.as_deref().cloned(),
                }
            }
            MethodBodyKind::BuiltIn(body) => FunctionShape {
                type_parameters: built_in_type_parameters(&self.heap, body.type_parameters),
                parameters: built_in_parameters(&self.heap, body.parameters),
                return_type: Some(descriptor_from_built_in_spec(&self.heap, &body.return_spec)),
            },
        }
    }

    fn class_shape(&self, id: ClassId) -> ClassShape {
        let class = &self.tables.classes[id.0 as usize];
        let mut methods = HashMap::new();
        for (name, entry) in class.methods() {
            if entry.declaring_class != id || generated_enum_method(class.kind, name) {
                continue;
            }

            methods.insert(
                name.clone(),
                MethodShape {
                    visibility: entry.visibility,
                    is_static: entry.is_static,
                    is_abstract: entry.is_abstract,
                    is_final: entry.is_final,
                    function: self.external_method_shape(entry.body),
                },
            );
        }

        for ((owner, name), entry) in &class.private_methods {
            if *owner != id {
                continue;
            }

            methods.insert(
                name.clone(),
                MethodShape {
                    visibility: entry.visibility,
                    is_static: entry.is_static,
                    is_abstract: entry.is_abstract,
                    is_final: entry.is_final,
                    function: self.external_method_shape(entry.body),
                },
            );
        }

        let mut properties = HashMap::new();
        for property in &class.slots {
            if property.declaring_class == id
                && !generated_enum_property(class.kind, &property.name)
            {
                properties.insert(
                    property.name.clone(),
                    PropertyShape {
                        visibility: property.visibility,
                        is_static: false,
                        is_readonly: property.is_readonly,
                        declared_type: property.declared_type.clone(),
                    },
                );
            }
        }

        for property in &class.statics_info {
            if property.declaring_class == id {
                properties.insert(
                    property.name.clone(),
                    PropertyShape {
                        visibility: property.visibility,
                        is_static: true,
                        is_readonly: property.is_readonly,
                        declared_type: property.declared_type.clone(),
                    },
                );
            }
        }

        let constants = class
            .constants()
            .filter(|(_, constant)| constant.declaring_class == id)
            .map(|(name, constant)| {
                (
                    name.clone(),
                    ConstantShape {
                        visibility: constant.visibility,
                        declared_type: constant.declared_type.clone(),
                    },
                )
            })
            .collect();

        ClassShape {
            kind: class.kind,
            is_abstract: class.is_abstract,
            is_final: class.is_final,
            is_readonly: class.is_readonly,
            type_parameters: class.type_parameters.to_vec(),
            direct_bases: class.direct_bases.clone(),
            constants,
            properties,
            methods,
            cases: class
                .enum_cases
                .iter()
                .map(|case| case.name.clone())
                .collect(),
            sealed_to: class.sealed_to.clone(),
            attribute_flags: class.attribute_flags,
        }
    }

    fn check_function_shape(
        &mut self,
        expected: &CompiledFunction,
        actual: &FunctionShape,
        name: &Atom,
        span: Span,
        unit: &CompiledUnit,
    ) -> Result<(), VirtualMachineControl> {
        self.check_type_parameters(
            &expected.type_parameters,
            &actual.type_parameters,
            name,
            span,
            unit,
        )?;

        if expected.parameters.len() != actual.parameters.len() {
            return Err(self.external_mismatch(name, span, unit, "the signature does not match"));
        }

        for (expected, actual) in expected.parameters.iter().zip(&actual.parameters) {
            if expected.name != actual.name
                || expected.has_default != actual.has_default
                || expected.sensitive != actual.sensitive
                || !self.optional_descriptors_equivalent(
                    expected.declared_type.as_ref(),
                    actual.declared_type.as_ref(),
                )?
            {
                return Err(self.external_mismatch(
                    name,
                    span,
                    unit,
                    "the signature does not match",
                ));
            }
        }

        let constructor_without_written_return = name.as_bytes().ends_with(b"::__construct")
            && expected.return_type.is_none()
            && matches!(actual.return_type, Some(TypeDescriptor::Void));
        if !constructor_without_written_return
            && !self.optional_descriptors_equivalent(
                expected.return_type.as_ref(),
                actual.return_type.as_ref(),
            )?
        {
            return Err(self.external_mismatch(name, span, unit, "the signature does not match"));
        }

        Ok(())
    }

    fn check_type_parameters(
        &mut self,
        expected: &[CompiledTypeParameter],
        actual: &[CompiledTypeParameter],
        name: &Atom,
        span: Span,
        unit: &CompiledUnit,
    ) -> Result<(), VirtualMachineControl> {
        if expected.len() != actual.len() {
            return Err(self.external_mismatch(name, span, unit, "type parameters do not match"));
        }

        for (expected, actual) in expected.iter().zip(actual) {
            if expected.name != actual.name
                || expected.variance != actual.variance
                || expected.bounds.len() != actual.bounds.len()
                || expected.default.is_some() != actual.default.is_some()
            {
                return Err(self.external_mismatch(
                    name,
                    span,
                    unit,
                    "type parameters do not match",
                ));
            }

            for (expected, actual) in expected.bounds.iter().zip(&actual.bounds) {
                if !self.link_descriptors_equivalent(expected, actual)? {
                    return Err(self.external_mismatch(
                        name,
                        span,
                        unit,
                        "type parameter bounds do not match",
                    ));
                }
            }

            if let (Some(expected), Some(actual)) = (&expected.default, &actual.default)
                && !self.link_descriptors_equivalent(expected, actual)?
            {
                return Err(self.external_mismatch(
                    name,
                    span,
                    unit,
                    "type parameter defaults do not match",
                ));
            }
        }

        Ok(())
    }

    fn check_bases(
        &mut self,
        expected: &CompiledClassLike,
        actual: &[RuntimeBase],
        unit: &CompiledUnit,
    ) -> Result<(), VirtualMachineControl> {
        let mut expected_bases = Vec::new();
        if let Some(parent) = &expected.parent {
            expected_bases.push(parent);
        }

        expected_bases.extend(&expected.interfaces);
        if expected_bases.len() != actual.len() {
            return Err(self.external_mismatch(
                &expected.name,
                expected.span,
                unit,
                "base declarations do not match",
            ));
        }

        for (expected_base, actual_base) in expected_bases.into_iter().zip(actual) {
            let actual_class = &self.tables.classes[actual_base.class.0 as usize];
            if expected_base.name != actual_class.name
                || expected_base.type_arguments.is_some() != actual_base.type_arguments.is_some()
            {
                return Err(self.external_mismatch(
                    &expected.name,
                    expected.span,
                    unit,
                    "base declarations do not match",
                ));
            }

            if let (Some(expected_arguments), Some(actual_arguments)) =
                (&expected_base.type_arguments, &actual_base.type_arguments)
            {
                if expected_arguments.len() != actual_arguments.len() {
                    return Err(self.external_mismatch(
                        &expected.name,
                        expected.span,
                        unit,
                        "base type arguments do not match",
                    ));
                }

                for (expected_argument, actual_argument) in
                    expected_arguments.iter().zip(actual_arguments)
                {
                    if !self.link_descriptors_equivalent(expected_argument, actual_argument)? {
                        return Err(self.external_mismatch(
                            &expected.name,
                            expected.span,
                            unit,
                            "base type arguments do not match",
                        ));
                    }
                }
            }
        }

        Ok(())
    }

    fn optional_descriptors_equivalent(
        &mut self,
        expected: Option<&TypeDescriptor>,
        actual: Option<&TypeDescriptor>,
    ) -> Result<bool, VirtualMachineControl> {
        match (expected, actual) {
            (None, None) => Ok(true),
            (Some(expected), Some(actual)) => self.link_descriptors_equivalent(expected, actual),
            _ => Ok(false),
        }
    }

    fn external_mismatch(
        &mut self,
        name: &Atom,
        span: Span,
        unit: &CompiledUnit,
        detail: &str,
    ) -> VirtualMachineControl {
        self.linker_error_at(
            &unit.path,
            span,
            format!("the external declaration {name} does not match: {detail}"),
        )
    }
}

const fn symbol_kind(kind: ClassLikeKind) -> SymbolKind {
    match kind {
        ClassLikeKind::Class => SymbolKind::Class,
        ClassLikeKind::Interface => SymbolKind::Interface,
        ClassLikeKind::Enum => SymbolKind::Enum,
    }
}

fn generated_enum_method(kind: ClassLikeKind, name: &Atom) -> bool {
    kind == ClassLikeKind::Enum && matches!(name.as_bytes(), b"cases" | b"from" | b"tryFrom")
}

fn generated_enum_property(kind: ClassLikeKind, name: &Atom) -> bool {
    kind == ClassLikeKind::Enum && matches!(name.as_bytes(), b"name" | b"value")
}
