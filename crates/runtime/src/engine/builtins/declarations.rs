//! Registering built-in interfaces, classes, and enums from their specs.

use std::rc::Rc;
use std::str;

use crate::builtin::spec::BaseSpec;
use crate::builtin::spec::ClassConstantSpec;
use crate::builtin::spec::ClassSpec;
use crate::builtin::spec::EnumSpec;
use crate::builtin::spec::InterfaceMethodSpec;
use crate::builtin::spec::InterfaceSpec;
use crate::builtin::spec::MethodSpec;
use crate::builtin::spec::PropertySpec;
use crate::builtin::spec::TypeParameterSpec;
use crate::builtin::spec::TypeSpec;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::BuiltInCallableAttributes;
use crate::bytecode::unit::ClassLikeKind;
use crate::bytecode::unit::EnumBacking;
use crate::bytecode::unit::Visibility;
use crate::classes::BuiltInMethodBody;
use crate::classes::ClassConstantEntry;
use crate::classes::ClassConstantValue;
use crate::classes::ClassMemberEntry;
use crate::classes::EnumCaseDeclaration;
use crate::classes::MethodBodyKind;
use crate::classes::MethodEntry;
use crate::classes::PropertyInfo;
use crate::classes::RuntimeBase;
use crate::classes::RuntimeClass;
use crate::core::classes;
use crate::engine::tables::RuntimeTables;
use crate::linker::descriptors::descriptor_from_built_in_spec;
use crate::symbols::FunctionTable;
use crate::symbols::SymbolEntry;
use crate::symbols::SymbolKind;
use crate::value::atom::Atom;
use crate::value::heap::Heap;
use crate::value::object::ClassId;

use crate::engine::builtins::built_in_type_parameters;
use crate::engine::builtins::constant_spec_value;
use crate::u32_index;
use crate::unwrap_result_invariant;

pub(in crate::engine::builtins) fn intersect_permissions(
    current: &mut Option<Vec<Atom>>,
    inherited: &[Atom],
) {
    match current {
        Some(permitted) => permitted.retain(|name| inherited.contains(name)),
        None => *current = Some(inherited.to_vec()),
    }
}

pub(in crate::engine::builtins) fn base_arguments(
    heap: &Heap,
    base: &BaseSpec,
) -> Option<Vec<TypeDescriptor>> {
    base.arguments.map(|arguments| {
        arguments
            .iter()
            .map(|argument| descriptor_from_built_in_spec(heap, argument))
            .collect()
    })
}

fn install_property(
    class: &mut RuntimeClass,
    heap: &Heap,
    declaring_class: ClassId,
    property: &PropertySpec,
) {
    let name = heap.intern(property.name.as_bytes());
    let slot = u32_index(class.slots.len());
    class.slot_names.insert(name.clone(), slot);
    class.append_property(PropertyInfo {
        name,
        visibility: property.visibility,
        is_readonly: property.is_readonly,
        declaring_class,
        default: None,
        declared_type: property
            .type_spec
            .as_ref()
            .map(|spec| descriptor_from_built_in_spec(heap, spec)),
    });
}

fn install_constant(
    class: &mut RuntimeClass,
    heap: &Heap,
    declaring_class: ClassId,
    constant: &ClassConstantSpec,
) {
    class.members.insert(
        heap.intern(constant.name.as_bytes()),
        ClassMemberEntry::Constant(ClassConstantEntry {
            value: ClassConstantValue::Evaluated(constant_spec_value(heap, &constant.value)),
            declared_type: Some(descriptor_from_built_in_spec(heap, &constant.type_spec)),
            visibility: constant.visibility,
            declaring_class,
        }),
    );
}

fn method_body(owner: &str, method: &MethodSpec) -> BuiltInMethodBody {
    BuiltInMethodBody {
        handler: method.handler,
        type_parameters: method.type_parameters,
        parameters: method.parameters,
        return_spec: method.return_spec,
        signature: method.signature,
        attributes: BuiltInCallableAttributes::resolve(owner, method.markers),
    }
}

fn interface_method_body(owner: &str, method: &InterfaceMethodSpec) -> MethodBodyKind {
    MethodBodyKind::BuiltIn(BuiltInMethodBody {
        handler: method
            .default_handler
            .unwrap_or(classes::abstract_method_body),
        type_parameters: method.type_parameters,
        parameters: method.parameters,
        return_spec: method.return_spec,
        signature: method.signature,
        attributes: BuiltInCallableAttributes::resolve(owner, method.markers),
    })
}

fn check_builtin_destructor(
    owner: &str,
    name: &str,
    visibility: Visibility,
    is_static: bool,
    has_type_parameters: bool,
    has_parameters: bool,
    return_spec: TypeSpec,
) {
    if name != "__destruct" {
        return;
    }
    let valid = visibility == Visibility::Public
        && !is_static
        && !has_type_parameters
        && !has_parameters
        && matches!(return_spec, TypeSpec::Mixed | TypeSpec::Void);
    assert!(
        valid,
        "{owner}::__destruct must be public, non-static, non-generic, parameterless, and return void or omit its return type"
    );
}

fn install_enum_members(class: &mut RuntimeClass, heap: &Heap, id: ClassId, spec: &EnumSpec) {
    let name_slot = u32_index(class.slots.len());
    class.slot_names.insert(heap.intern(b"name"), name_slot);
    class.append_property(PropertyInfo {
        name: heap.intern(b"name"),
        visibility: Visibility::Public,
        is_readonly: true,
        declaring_class: id,
        default: None,
        declared_type: Some(TypeDescriptor::String),
    });

    if let Some(backing) = spec.backing {
        let value_slot = u32_index(class.slots.len());
        class.slot_names.insert(heap.intern(b"value"), value_slot);
        class.append_property(PropertyInfo {
            name: heap.intern(b"value"),
            visibility: Visibility::Public,
            is_readonly: true,
            declaring_class: id,
            default: None,
            declared_type: Some(match backing {
                EnumBacking::Int => TypeDescriptor::Int,
                EnumBacking::String => TypeDescriptor::String,
            }),
        });
    }

    for case in spec.cases {
        let case_name = heap.intern(case.name.as_bytes());
        let position = u32_index(class.enum_cases.len());
        class.enum_cases.push(EnumCaseDeclaration {
            name: case_name.clone(),
            backing: case
                .value
                .as_ref()
                .map(|value| constant_spec_value(heap, value)),
        });
        class
            .members
            .insert(case_name, ClassMemberEntry::EnumCase(position));
    }
}

fn install_enum_methods(class: &mut RuntimeClass, heap: &Heap, id: ClassId, spec: &EnumSpec) {
    for method in &spec.methods {
        check_builtin_destructor(
            spec.name,
            method.name,
            method.visibility,
            method.is_static,
            !method.type_parameters.is_empty(),
            !method.parameters.is_empty(),
            method.return_spec,
        );
        class.members.insert(
            heap.intern(method.name.as_bytes()),
            ClassMemberEntry::Method(MethodEntry {
                visibility: method.visibility,
                is_static: method.is_static,
                is_abstract: false,
                is_final: false,
                declaring_class: id,
                body: MethodBodyKind::BuiltIn(method_body(spec.name, method)),
            }),
        );
    }

    let cases_name = heap.intern(b"cases");
    assert!(
        !class.members.contains_key(&cases_name),
        "the enum {} cannot redeclare the built-in method cases",
        spec.name
    );
    class.members.insert(
        cases_name,
        ClassMemberEntry::Method(MethodEntry {
            visibility: Visibility::Public,
            is_static: true,
            is_abstract: false,
            is_final: true,
            declaring_class: id,
            body: MethodBodyKind::BuiltIn(classes::enum_cases_body()),
        }),
    );

    if spec.backing.is_none() {
        return;
    }
    for (method_name, body) in [
        (b"from".as_slice(), classes::enum_from_body()),
        (b"tryFrom".as_slice(), classes::enum_try_from_body()),
    ] {
        let method_name = heap.intern(method_name);
        assert!(
            !class.members.contains_key(&method_name),
            "the enum {} cannot redeclare the built-in method {method_name}",
            spec.name
        );
        class.members.insert(
            method_name,
            ClassMemberEntry::Method(MethodEntry {
                visibility: Visibility::Public,
                is_static: true,
                is_abstract: false,
                is_final: true,
                declaring_class: id,
                body: MethodBodyKind::BuiltIn(body),
            }),
        );
    }
}

impl RuntimeTables {
    fn check_sealed(
        &self,
        base: ClassId,
        base_name: &'static str,
        name: &Atom,
        spec_name: &'static str,
        action: &'static str,
    ) {
        if let Some(permitted) = &self.classes[base.0 as usize].sealed_to
            && !permitted.contains(name)
        {
            panic!("{base_name} is sealed and does not permit {spec_name} to {action} it");
        }
    }

    fn finish_builtin_class(
        &mut self,
        name: Atom,
        id: ClassId,
        mut class: RuntimeClass,
        kind: SymbolKind,
    ) {
        class.finalize_layout(&self.destructor_name);
        self.has_destructor_classes |= class.destructor.is_some();
        self.classes.push(class);
        self.symbols.insert(
            name,
            SymbolEntry {
                kind,
                index: id.0,
                table: FunctionTable::User,
            },
        );
    }

    fn start_builtin_class(
        &self,
        heap: &Heap,
        name: &'static str,
        kind: ClassLikeKind,
        type_parameters: &'static [TypeParameterSpec],
        sealed_to: Option<&'static [&'static str]>,
    ) -> (Atom, ClassId, RuntimeClass) {
        let name_atom = heap.intern(name.as_bytes());
        self.check_core_name(&name_atom);
        let id = ClassId(u32_index(self.classes.len()));
        let mut class = RuntimeClass::new(name_atom.clone(), kind);
        class.type_parameter_arity = Some((
            u32_index(
                type_parameters
                    .iter()
                    .position(|parameter| parameter.default.is_some())
                    .unwrap_or(type_parameters.len()),
            ),
            u32_index(type_parameters.len()),
        ));
        class.type_parameters = Rc::from(built_in_type_parameters(heap, type_parameters));
        class.sealed_to = sealed_to.map(|names| {
            names
                .iter()
                .map(|name| heap.intern(name.as_bytes()))
                .collect()
        });
        (name_atom, id, class)
    }

    /// Resolves a built-in parent or interface name against the registries.
    pub(in crate::engine::builtins) fn resolve_builtin_reference(
        &self,
        heap: &Heap,
        name: &'static str,
        expected: SymbolKind,
    ) -> ClassId {
        let atom = heap.intern(name.as_bytes());
        let entry = self.symbols.get(&atom).copied();
        match entry {
            Some(entry) if entry.kind == expected => ClassId(entry.index),
            _ => panic!("the generated core references {name}, which is not declared"),
        }
    }

    pub(in crate::engine::builtins) fn register_builtin_interface(
        &mut self,
        heap: &Heap,
        spec: &InterfaceSpec,
    ) {
        let (name, id, mut class) = self.start_builtin_class(
            heap,
            spec.name,
            ClassLikeKind::Interface,
            spec.type_parameters,
            spec.sealed_to,
        );
        class.is_abstract = true;

        for extended in spec.extends {
            let parent = self.resolve_builtin_reference(heap, extended.name, SymbolKind::Interface);
            if let Some(inherited) = &self.classes[parent.0 as usize].sealed_to {
                intersect_permissions(&mut class.sealed_to, inherited);
            }

            class.insert_interface(parent);

            class.direct_bases.push(RuntimeBase {
                class: parent,
                type_arguments: base_arguments(heap, extended),
            });

            for grand in &self.classes[parent.0 as usize].interfaces {
                class.insert_interface(*grand);
            }

            for (method_name, entry) in self.classes[parent.0 as usize].methods() {
                class
                    .members
                    .entry(method_name.clone())
                    .or_insert(ClassMemberEntry::Method(entry));
            }
        }

        for method in &spec.methods {
            check_builtin_destructor(
                spec.name,
                method.name,
                Visibility::Public,
                method.is_static,
                !method.type_parameters.is_empty(),
                !method.parameters.is_empty(),
                method.return_spec,
            );
            let body = interface_method_body(spec.name, method);

            class.members.insert(
                heap.intern(method.name.as_bytes()),
                ClassMemberEntry::Method(MethodEntry {
                    visibility: Visibility::Public,
                    is_static: method.is_static,
                    is_abstract: method.default_handler.is_none(),
                    is_final: false,
                    declaring_class: id,
                    body,
                }),
            );
        }

        for property in spec.properties {
            install_property(&mut class, heap, id, property);
        }

        for constant in spec.constants {
            install_constant(&mut class, heap, id, constant);
        }

        self.finish_builtin_class(name, id, class, SymbolKind::Interface);
    }

    pub(in crate::engine::builtins) fn register_builtin_class(
        &mut self,
        heap: &Heap,
        spec: &ClassSpec,
    ) {
        let (name, id, mut class) = self.start_builtin_class(
            heap,
            spec.name,
            ClassLikeKind::Class,
            spec.type_parameters,
            spec.sealed_to,
        );
        class.is_final = spec.is_final;
        class.is_abstract = spec.is_abstract;
        class.is_readonly = spec.is_readonly;
        if let Some(parent_spec) = spec.parent {
            self.inherit_builtin_class_parent(&mut class, heap, &name, spec.name, &parent_spec);
        }
        self.implement_builtin_class_interfaces(&mut class, heap, &name, spec);

        for property in spec.properties {
            install_property(&mut class, heap, id, property);
        }
        for method in &spec.methods {
            check_builtin_destructor(
                spec.name,
                method.name,
                method.visibility,
                method.is_static,
                !method.type_parameters.is_empty(),
                !method.parameters.is_empty(),
                method.return_spec,
            );
            let method_name = heap.intern(method.name.as_bytes());
            let entry = MethodEntry {
                visibility: method.visibility,
                is_static: method.is_static,
                is_abstract: false,
                is_final: false,
                declaring_class: id,
                body: MethodBodyKind::BuiltIn(method_body(spec.name, method)),
            };
            if method.visibility == Visibility::Private {
                class
                    .private_methods
                    .insert((id, method_name.clone()), entry);
            }
            class
                .members
                .insert(method_name, ClassMemberEntry::Method(entry));
        }
        for constant in spec.constants {
            install_constant(&mut class, heap, id, constant);
        }
        match (spec.built_in_hooks, spec.built_in_initializer) {
            (Some(hooks), Some(initializer)) => {
                let mut state_hooks = class.built_in_state_hooks.to_vec();
                state_hooks.push(hooks);
                class.built_in_state_hooks = state_hooks.into();

                let mut state_initializers = class.built_in_state_initializers.to_vec();
                state_initializers.push(initializer);
                class.built_in_state_initializers = state_initializers.into();
            }
            (None, None) => {}
            _ => panic!(
                "{} must declare built-in hooks and an initializer together",
                spec.name
            ),
        }
        class.attribute_flags = spec.attribute_flags;
        self.finish_builtin_class(name, id, class, SymbolKind::Class);
    }

    fn inherit_builtin_class_parent(
        &self,
        class: &mut RuntimeClass,
        heap: &Heap,
        name: &Atom,
        spec_name: &'static str,
        parent_spec: &BaseSpec,
    ) {
        let parent = self.resolve_builtin_reference(heap, parent_spec.name, SymbolKind::Class);
        self.check_sealed(parent, parent_spec.name, name, spec_name, "extend");

        let parent_class = &self.classes[parent.0 as usize];
        class.parent = Some(parent);
        class.direct_bases.push(RuntimeBase {
            class: parent,
            type_arguments: base_arguments(heap, parent_spec),
        });
        class.inherit_interfaces(parent_class);
        class.inherit_properties(parent_class);
        class.members.clone_from(&parent_class.members);
        class
            .private_methods
            .clone_from(&parent_class.private_methods);
        class.built_in_state_hooks = Rc::clone(&parent_class.built_in_state_hooks);
        class.built_in_state_initializers = Rc::clone(&parent_class.built_in_state_initializers);
    }

    fn implement_builtin_class_interfaces(
        &self,
        class: &mut RuntimeClass,
        heap: &Heap,
        name: &Atom,
        spec: &ClassSpec,
    ) {
        for interface_spec in spec.interfaces {
            let interface =
                self.resolve_builtin_reference(heap, interface_spec.name, SymbolKind::Interface);
            self.check_sealed(interface, interface_spec.name, name, spec.name, "implement");

            class.insert_interface(interface);
            class.direct_bases.push(RuntimeBase {
                class: interface,
                type_arguments: base_arguments(heap, interface_spec),
            });

            for inherited in &self.classes[interface.0 as usize].interfaces {
                class.insert_interface(*inherited);
            }
            for (method_name, entry) in self.classes[interface.0 as usize]
                .methods()
                .filter(|(_, entry)| !entry.is_abstract)
            {
                class
                    .members
                    .entry(method_name.clone())
                    .or_insert(ClassMemberEntry::Method(entry));
            }
        }
    }

    pub(in crate::engine::builtins) fn register_builtin_enum(
        &mut self,
        heap: &Heap,
        spec: &EnumSpec,
    ) {
        let name = heap.intern(spec.name.as_bytes());
        self.check_core_name(&name);
        let id = ClassId(u32_index(self.classes.len()));
        let mut class = RuntimeClass::new(name.clone(), ClassLikeKind::Enum);
        class.is_abstract = true;
        self.implement_builtin_enum_interfaces(&mut class, heap, &name, spec);
        self.install_builtin_enum_protocol(&mut class, heap, spec.backing);
        install_enum_members(&mut class, heap, id, spec);
        install_enum_methods(&mut class, heap, id, spec);

        for constant in spec.constants {
            install_constant(&mut class, heap, id, constant);
        }

        self.finish_builtin_class(name, id, class, SymbolKind::Enum);
    }

    fn implement_builtin_enum_interfaces(
        &self,
        class: &mut RuntimeClass,
        heap: &Heap,
        name: &Atom,
        spec: &EnumSpec,
    ) {
        for interface_spec in spec.interfaces {
            let interface =
                self.resolve_builtin_reference(heap, interface_spec.name, SymbolKind::Interface);
            self.check_sealed(interface, interface_spec.name, name, spec.name, "implement");

            class.insert_interface(interface);
            for inherited in &self.classes[interface.0 as usize].interfaces {
                class.insert_interface(*inherited);
            }
            class.direct_bases.push(RuntimeBase {
                class: interface,
                type_arguments: base_arguments(heap, interface_spec),
            });
        }
    }

    fn install_builtin_enum_protocol(
        &self,
        class: &mut RuntimeClass,
        heap: &Heap,
        backing: Option<EnumBacking>,
    ) {
        let (protocol_name, type_arguments) = match backing {
            None => (classes::names::UNIT_ENUM, None),
            Some(EnumBacking::Int) => {
                (classes::names::BACKED_ENUM, Some(vec![TypeDescriptor::Int]))
            }
            Some(EnumBacking::String) => (
                classes::names::BACKED_ENUM,
                Some(vec![TypeDescriptor::String]),
            ),
        };
        // SAFETY: protocol names contain ASCII.
        let protocol_text = unsafe {
            unwrap_result_invariant(
                str::from_utf8(protocol_name),
                "well-known enum protocol names are UTF-8",
            )
        };
        let protocol = self.resolve_builtin_reference(heap, protocol_text, SymbolKind::Interface);
        class.direct_bases.push(RuntimeBase {
            class: protocol,
            type_arguments,
        });
        class.insert_interface(protocol);
        for inherited in &self.classes[protocol.0 as usize].interfaces {
            class.insert_interface(*inherited);
        }
    }
}
