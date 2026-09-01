//! Linking a unit's classes: dependency order, base resolution, slot layout,
//! and method flattening.

use std::collections::VecDeque;
use std::mem;
use std::rc::Rc;

use hashbrown::HashMap;
use hashbrown::hash_map::Entry;

use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::unit::CONSISTENT_CONSTRUCTOR_ATTRIBUTE;
use crate::bytecode::unit::CONSISTENT_GENERICS_ATTRIBUTE;
use crate::bytecode::unit::ClassLikeKind;
use crate::bytecode::unit::CompiledBaseReference;
use crate::bytecode::unit::CompiledClassConstant;
use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::unit::CompiledEnumCase;
use crate::bytecode::unit::CompiledMethod;
use crate::bytecode::unit::CompiledProperty;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::unit::ConstantInitializer;
use crate::bytecode::unit::EnumBacking;
use crate::bytecode::unit::Visibility;
use crate::bytecode::unit::is_external;
use crate::bytecode::unit::literal_value;
use crate::classes::BuiltInMethodBody;
use crate::classes::ClassConstantEntry;
use crate::classes::ClassConstantValue;
use crate::classes::ClassMemberEntry;
use crate::classes::EnumCaseDeclaration;
use crate::classes::MethodBodyKind;
use crate::classes::MethodEntry;
use crate::classes::PropertyDefault;
use crate::classes::PropertyInfo;
use crate::classes::RuntimeBase;
use crate::classes::RuntimeClass;
use crate::core::classes;
use crate::engine::Engine;
use crate::engine::builtins::binder_arity_of;
use crate::engine::builtins::runtime_function;
use crate::linker::OverrideCheck;
use crate::linker::Replaced;
use crate::linker::SlotPlacement;
use crate::linker::slot_placement;
use crate::symbols::FunctionLocator;
use crate::symbols::FunctionTable;
use crate::symbols::SymbolEntry;
use crate::symbols::SymbolKind;
use crate::symbols::UnitContext;
use crate::u32_index;
use crate::unwrap_option_invariant;
use crate::value::Value;
use crate::value::ValueView;
use crate::value::atom::Atom;
use crate::value::dict::DictObject;
use crate::value::function::FuncId;
use crate::value::heap::Heap;
use crate::value::object::ClassId;
use crate::value::vec::VecObject;
use crate::vm::VirtualMachineControl;

struct ClassLink<'a> {
    compiled: &'a CompiledClassLike,
    unit: &'a CompiledUnit,
    context: &'a Rc<UnitContext>,
    class_index: usize,
    id: ClassId,
    name: &'a str,
    parent: Option<ClassId>,
    direct_interfaces: &'a [ClassId],
}

#[derive(Default)]
struct EnumBackings {
    integers: HashMap<i64, Atom>,
    strings: HashMap<Vec<u8>, Atom>,
}

impl EnumBackings {
    fn insert(&mut self, backing: &Value, case: &Atom) -> Option<Atom> {
        match backing.transparent() {
            ValueView::Int(value) => match self.integers.entry(*value) {
                Entry::Occupied(entry) => Some(entry.get().clone()),
                Entry::Vacant(entry) => {
                    entry.insert(case.clone());
                    None
                }
            },
            _ => match self.strings.entry(backing.as_string_bytes()?.to_vec()) {
                Entry::Occupied(entry) => Some(entry.get().clone()),
                Entry::Vacant(entry) => {
                    entry.insert(case.clone());
                    None
                }
            },
        }
    }
}

impl Engine {
    /// Links a unit's classes: within the unit, parents and interfaces link
    /// before their dependents; a dependency that is neither in the unit nor
    /// already declared is a link error, and no progress over a non-empty
    /// remainder is a cycle.
    pub(crate) fn link_unit_classes(
        &mut self,
        unit: &Rc<CompiledUnit>,
        context: &Rc<UnitContext>,
    ) -> Result<(), VirtualMachineControl> {
        let local_classes: Vec<usize> = unit
            .classes
            .iter()
            .enumerate()
            .filter(|(_, class)| !is_external(&class.attributes))
            .map(|(index, _)| index)
            .collect();
        let local_indexes: HashMap<Atom, usize> = local_classes
            .iter()
            .map(|&index| (unit.classes[index].name.clone(), index))
            .collect();
        let mut pending_dependencies = vec![0usize; unit.classes.len()];
        let mut dependents = vec![Vec::new(); unit.classes.len()];

        for &class_index in &local_classes {
            let compiled = &unit.classes[class_index];
            for base in compiled.parent.iter().chain(&compiled.interfaces) {
                let Some(&base_index) = local_indexes.get(&base.name) else {
                    continue;
                };

                pending_dependencies[class_index] += 1;
                dependents[base_index].push(class_index);
            }
        }

        let mut ready: VecDeque<usize> = local_classes
            .iter()
            .copied()
            .filter(|&index| pending_dependencies[index] == 0)
            .collect();
        let mut rounds = vec![0usize; unit.classes.len()];
        let mut resolved = 0usize;

        while let Some(base_index) = ready.pop_front() {
            resolved += 1;
            for &dependent in &dependents[base_index] {
                let round = rounds[base_index] + usize::from(base_index > dependent);
                rounds[dependent] = rounds[dependent].max(round);
                pending_dependencies[dependent] -= 1;
                if pending_dependencies[dependent] == 0 {
                    ready.push_back(dependent);
                }
            }
        }

        if resolved != local_classes.len() {
            let class_index = local_classes
                .iter()
                .copied()
                .find(|&index| pending_dependencies[index] != 0);

            // SAFETY: an unresolved class must exist when the resolved count differs.
            let class_index = unsafe {
                unwrap_option_invariant(class_index, "an inheritance cycle has an unresolved class")
            };

            let declaration = &unit.classes[class_index];
            let name = declaration.name.to_string_lossy().into_owned();
            let kind = match declaration.kind {
                ClassLikeKind::Class => "class",
                ClassLikeKind::Interface => "interface",
                ClassLikeKind::Enum => "enum",
            };

            return Err(self.linker_error_at(
                &unit.path,
                declaration.span,
                format!("the {kind} {name} participates in cyclic inheritance"),
            ));
        }

        let highest_round = local_classes
            .iter()
            .map(|&index| rounds[index])
            .max()
            .unwrap_or(0);

        let mut classes_by_round = vec![Vec::new(); highest_round + 1];
        for class_index in local_classes {
            classes_by_round[rounds[class_index]].push(class_index);
        }

        for class_index in classes_by_round.into_iter().flatten() {
            self.link_class(&unit.classes[class_index], unit, context, class_index)?;
        }

        for compiled in &unit.classes {
            if is_external(&compiled.attributes) {
                continue;
            }
            // SAFETY: the preceding loop linked every non-external class.
            let entry = unsafe {
                unwrap_option_invariant(
                    self.tables.symbols.get(&compiled.name).copied(),
                    "every class in the unit was linked",
                )
            };

            let id = ClassId(entry.index);
            let class = mem::replace(
                &mut self.tables.classes[id.0 as usize],
                RuntimeClass::new(compiled.name.clone(), compiled.kind),
            );

            let name_text = compiled.name.to_string_lossy().into_owned();
            self.tables.symbols.remove(&compiled.name);

            let validation =
                self.validate_linked_class_contracts(&class, id, compiled, &name_text, &unit.path);
            self.tables.classes[id.0 as usize] = class;
            self.tables.symbols.insert(compiled.name.clone(), entry);
            validation?;
        }

        self.validate_unit_attributes(unit)?;
        Ok(())
    }

    fn validate_linked_class_contracts(
        &mut self,
        class: &RuntimeClass,
        id: ClassId,
        compiled: &CompiledClassLike,
        name_text: &str,
        path: &Atom,
    ) -> Result<(), VirtualMachineControl> {
        if let Some(parent_id) = class.parent {
            for method in &compiled.methods {
                let Some(inherited) =
                    self.tables.classes[parent_id.0 as usize].method(&method.name)
                else {
                    continue;
                };

                if inherited.visibility == Visibility::Private {
                    continue;
                }

                // SAFETY: linking inserted every declared method.
                let replacement = unsafe {
                    unwrap_option_invariant(
                        class.method(&method.name),
                        "a linked method is present in its class method table",
                    )
                };

                self.check_override(&OverrideCheck {
                    current: class,
                    current_id: id,
                    method_name: &method.name,
                    replacement: &replacement,
                    replaced: &inherited,
                    name_text,
                    source: Replaced::Inherited,
                    enforce_constructor: class.consistent_constructor,
                    path,
                })?;
            }
        }

        self.check_declared_contracts(class, id, compiled, name_text, path)
    }

    /// Links one class: parent and interface resolution with the kind,
    /// final, and sealed rules; slot layout with parent slots first; method
    /// flattening (own over inherited over interface defaults, with the
    /// conflict rule); and static, constant, and default initialization.
    fn link_class(
        &mut self,
        compiled: &CompiledClassLike,
        unit: &Rc<CompiledUnit>,
        context: &Rc<UnitContext>,
        class_index: usize,
    ) -> Result<(), VirtualMachineControl> {
        let id = ClassId(u32_index(self.tables.classes.len()));
        let name_text = compiled.name.to_string_lossy().into_owned();
        let (parent, direct_interfaces) =
            self.resolve_class_bases(compiled, &name_text, &unit.path)?;
        self.check_sealed_bases(compiled, parent, &direct_interfaces, &name_text, &unit.path)?;

        let link = ClassLink {
            compiled,
            unit,
            context,
            class_index,
            id,
            name: &name_text,
            parent,
            direct_interfaces: &direct_interfaces,
        };

        let mut class = self.initialize_class(&link);
        self.link_class_bases(&mut class, &link)?;
        self.check_parent_readonly(&class, &link)?;
        self.inherit_parent_members(&mut class, &link);
        self.inherit_interfaces(&mut class, &link);
        self.check_interface_member_kinds(&class, &link)?;
        self.link_properties(&mut class, &link)?;
        self.link_methods(&mut class, &link)?;
        self.link_enum_methods(&mut class, &link)?;
        self.inherit_interface_methods(&mut class, &link)?;
        self.inherit_interface_constants(&mut class, &link)?;
        self.link_constants(&mut class, &link)?;
        self.finish_class(class, &link)
    }

    fn resolve_class_bases(
        &mut self,
        compiled: &CompiledClassLike,
        name: &str,
        path: &Atom,
    ) -> Result<(Option<ClassId>, Vec<ClassId>), VirtualMachineControl> {
        let parent = match &compiled.parent {
            Some(base) => Some(self.resolve_parent(base, name, path)?),
            None => None,
        };

        let mut interfaces = Vec::with_capacity(compiled.interfaces.len());
        for base in &compiled.interfaces {
            interfaces.push(self.resolve_interface(base, name, path)?);
        }

        Ok((parent, interfaces))
    }

    fn check_sealed_bases(
        &mut self,
        compiled: &CompiledClassLike,
        parent: Option<ClassId>,
        interfaces: &[ClassId],
        name: &str,
        path: &Atom,
    ) -> Result<(), VirtualMachineControl> {
        if let Some(parent) = parent {
            let parent = &self.tables.classes[parent.0 as usize];
            if let Some(permitted) = &parent.sealed_to
                && !permitted.contains(&compiled.name)
            {
                let span = compiled
                    .parent
                    .as_ref()
                    .map_or(compiled.span, |base| base.span);
                return Err(self.linker_error_at(
                    path,
                    span,
                    format!(
                        "{} is sealed and does not permit {name} to extend it",
                        parent.name
                    ),
                ));
            }
        }

        let action = match compiled.kind {
            ClassLikeKind::Interface => "extend",
            _ => "implement",
        };

        for (base, interface) in compiled.interfaces.iter().zip(interfaces) {
            let interface = &self.tables.classes[interface.0 as usize];
            if let Some(permitted) = &interface.sealed_to
                && !permitted.contains(&compiled.name)
            {
                return Err(self.linker_error_at(
                    path,
                    base.span,
                    format!(
                        "{} is sealed and does not permit {name} to {action} it",
                        interface.name
                    ),
                ));
            }
        }

        Ok(())
    }

    fn initialize_class(&mut self, link: &ClassLink<'_>) -> RuntimeClass {
        self.tables.classes.push(RuntimeClass::new(
            link.compiled.name.clone(),
            link.compiled.kind,
        ));

        let mut class = RuntimeClass::new(link.compiled.name.clone(), link.compiled.kind);
        class.is_abstract =
            link.compiled.is_abstract || link.compiled.kind == ClassLikeKind::Interface;
        class.is_final = link.compiled.is_final;
        class.is_readonly = link.compiled.is_readonly;
        class.consistent_constructor = link
            .compiled
            .attributes
            .iter()
            .any(|attribute| attribute.class.as_bytes() == CONSISTENT_CONSTRUCTOR_ATTRIBUTE)
            || link.parent.is_some_and(|parent| {
                self.tables.classes[parent.0 as usize].consistent_constructor
            });

        class.consistent_generics = link
            .compiled
            .attributes
            .iter()
            .any(|attribute| attribute.class.as_bytes() == CONSISTENT_GENERICS_ATTRIBUTE)
            || link
                .parent
                .is_some_and(|parent| self.tables.classes[parent.0 as usize].consistent_generics);

        class.type_parameter_arity = Some(binder_arity_of(&link.compiled.type_parameters));
        class.type_parameters = Rc::from(link.compiled.type_parameters.as_slice());
        class.sealed_to = if link.compiled.kind == ClassLikeKind::Enum {
            None
        } else {
            link.compiled.sealed_to.clone()
        };

        class.parent = link.parent;
        class
    }

    fn finish_class(
        &mut self,
        mut class: RuntimeClass,
        link: &ClassLink<'_>,
    ) -> Result<(), VirtualMachineControl> {
        class.base_specializations =
            self.merge_base_specializations(&class, link.id, link.name, &link.unit.path)?;
        self.link_enum_members(&mut class, link)?;
        class.attribute_flags =
            self.extract_attribute_flags(&link.compiled.attributes, link.context, &link.unit.path)?;
        class
            .attribute_declarations
            .clone_from(&link.compiled.attributes);
        class.attribute_unit = Some(Rc::clone(link.context));
        class.finalize_layout(&self.tables.destructor_name);
        self.tables.has_destructor_classes |= class.destructor.is_some();
        let kind = match link.compiled.kind {
            ClassLikeKind::Class => SymbolKind::Class,
            ClassLikeKind::Interface => SymbolKind::Interface,
            ClassLikeKind::Enum => SymbolKind::Enum,
        };

        self.tables.classes[link.id.0 as usize] = class;
        self.tables.symbols.insert(
            link.compiled.name.clone(),
            SymbolEntry {
                kind,
                index: link.id.0,
                table: FunctionTable::User,
            },
        );

        Ok(())
    }

    fn link_class_bases(
        &mut self,
        class: &mut RuntimeClass,
        link: &ClassLink<'_>,
    ) -> Result<(), VirtualMachineControl> {
        if let (Some(base), Some(parent)) = (&link.compiled.parent, link.parent) {
            self.check_base_reference(class, link.id, base, parent, link.name, &link.unit.path)?;
            if class.consistent_generics {
                self.check_consistent_generic_bounds(
                    link.compiled,
                    base,
                    parent,
                    link.name,
                    &link.unit.path,
                )?;
            }

            class.direct_bases.push(RuntimeBase {
                class: parent,
                type_arguments: base.type_arguments.clone(),
            });
        }

        for (base, interface) in link.compiled.interfaces.iter().zip(link.direct_interfaces) {
            self.check_base_reference(
                class,
                link.id,
                base,
                *interface,
                link.name,
                &link.unit.path,
            )?;

            class.direct_bases.push(RuntimeBase {
                class: *interface,
                type_arguments: base.type_arguments.clone(),
            });
        }

        self.link_enum_protocol(class, link);
        Ok(())
    }

    fn link_enum_protocol(&self, class: &mut RuntimeClass, link: &ClassLink<'_>) {
        if link.compiled.kind != ClassLikeKind::Enum {
            return;
        }

        let (protocol, type_arguments) = match link.compiled.enum_backing {
            Some(EnumBacking::Int) => (
                self.tables.enum_classes.backed,
                Some(vec![TypeDescriptor::Int]),
            ),
            Some(EnumBacking::String) => (
                self.tables.enum_classes.backed,
                Some(vec![TypeDescriptor::String]),
            ),
            None => (self.tables.enum_classes.unit, None),
        };

        class.direct_bases.push(RuntimeBase {
            class: protocol,
            type_arguments,
        });

        class.insert_interface(protocol);
        for inherited in &self.tables.classes[protocol.0 as usize].interfaces {
            class.insert_interface(*inherited);
        }
    }

    fn check_parent_readonly(
        &mut self,
        class: &RuntimeClass,
        link: &ClassLink<'_>,
    ) -> Result<(), VirtualMachineControl> {
        let Some(parent) = link.parent else {
            return Ok(());
        };

        let parent = &self.tables.classes[parent.0 as usize];
        if class.is_readonly && !parent.is_readonly {
            return Err(self.linker_error_at(
                &link.unit.path,
                link.compiled.span,
                format!(
                    "the readonly class {} cannot extend {}, which is not readonly",
                    link.name, parent.name
                ),
            ));
        }

        if !class.is_readonly && parent.is_readonly {
            return Err(self.linker_error_at(
                &link.unit.path,
                link.compiled.span,
                format!(
                    "{} extends the readonly class {}, so it must be readonly too",
                    link.name, parent.name
                ),
            ));
        }

        Ok(())
    }

    fn inherit_parent_members(&self, class: &mut RuntimeClass, link: &ClassLink<'_>) {
        let Some(parent) = link.parent else {
            return;
        };

        let parent = &self.tables.classes[parent.0 as usize];
        class.inherit_interfaces(parent);
        class.inherit_properties(parent);
        class.members.clone_from(&parent.members);
        class.private_methods.clone_from(&parent.private_methods);
        class
            .built_in_state_hooks
            .clone_from(&parent.built_in_state_hooks);
        class
            .built_in_state_initializers
            .clone_from(&parent.built_in_state_initializers);
    }

    fn inherit_interfaces(&self, class: &mut RuntimeClass, link: &ClassLink<'_>) {
        for interface in link.direct_interfaces {
            class.insert_interface(*interface);

            for transitive in &self.tables.classes[interface.0 as usize].interfaces {
                class.insert_interface(*transitive);
            }
        }
    }

    fn check_interface_member_kinds(
        &mut self,
        class: &RuntimeClass,
        link: &ClassLink<'_>,
    ) -> Result<(), VirtualMachineControl> {
        let mut inherited_kinds = HashMap::new();
        for interface in link.direct_interfaces {
            let conflict = {
                let mut conflict = None;
                for (name, member) in &self.tables.classes[interface.0 as usize].members {
                    let inherited = member.kind();
                    let existing = class
                        .members
                        .get(name)
                        .map(ClassMemberEntry::kind)
                        .or_else(|| inherited_kinds.get(name).copied());
                    if let Some(existing) = existing
                        && existing != inherited
                    {
                        conflict = Some((name.clone(), inherited, existing));
                        break;
                    }

                    inherited_kinds.insert(name.clone(), inherited);
                }
                conflict
            };

            if let Some((name, inherited, existing)) = conflict {
                return Err(self.linker_error_at(
                    &link.unit.path,
                    link.compiled.span,
                    format!(
                        "{} inherits a {inherited} named {name}, but also inherits a \
                         {existing} of that name",
                        link.name
                    ),
                ));
            }
        }

        Ok(())
    }

    fn link_properties(
        &mut self,
        class: &mut RuntimeClass,
        link: &ClassLink<'_>,
    ) -> Result<(), VirtualMachineControl> {
        for (position, property) in link.compiled.properties.iter().enumerate() {
            if property.is_static {
                Self::link_static_property(class, link, property);
            } else {
                self.link_instance_property(class, link, property, position)?;
            }
        }

        Ok(())
    }

    fn link_static_property(
        class: &mut RuntimeClass,
        link: &ClassLink<'_>,
        property: &CompiledProperty,
    ) {
        let default = match &property.default {
            Some(ConstantInitializer::Literal(literal)) => literal_value(literal),
            Some(ConstantInitializer::Thunk(_)) | None => Value::uninitialized(),
        };

        let index = u32_index(class.statics.borrow().len());
        class.static_names.insert(property.name.clone(), index);
        class.statics.borrow_mut().push(default);
        class.statics_info.push(PropertyInfo {
            name: property.name.clone(),
            visibility: property.visibility,
            is_readonly: property.is_readonly,
            declaring_class: link.id,
            default: None,
            declared_type: property.declared_type.clone(),
        });
    }

    fn link_instance_property(
        &mut self,
        class: &mut RuntimeClass,
        link: &ClassLink<'_>,
        property: &CompiledProperty,
        position: usize,
    ) -> Result<(), VirtualMachineControl> {
        let default = property.default.as_ref().map(|initializer| {
            property_default_template(&self.heap, initializer).map_or_else(
                || PropertyDefault::Pending {
                    context: Rc::clone(link.context),
                    class_position: u32_index(link.class_index),
                    property_position: u32_index(position),
                },
                PropertyDefault::Value,
            )
        });

        let info = PropertyInfo {
            is_readonly: property.is_readonly || link.compiled.is_readonly,
            name: property.name.clone(),
            visibility: property.visibility,
            declaring_class: link.id,
            default,
            declared_type: property.declared_type.clone(),
        };

        let inherited = class
            .slot_names
            .get(&property.name)
            .copied()
            .map(|slot| (slot, class.slots[slot as usize].clone()));
        let placement = slot_placement(
            inherited
                .as_ref()
                .map(|(slot, property)| (*slot, property.visibility)),
            property.visibility,
        );

        match placement {
            SlotPlacement::Inherited(slot) => {
                // SAFETY: inherited placement is produced only from this value.
                let inherited = &unsafe {
                    unwrap_option_invariant(
                        inherited.as_ref(),
                        "inherited placement requires an inherited property",
                    )
                }
                .1;

                self.check_property_override(
                    class,
                    link.id,
                    &info,
                    inherited,
                    link.name,
                    &link.unit.path,
                )?;

                class.replace_property(slot, info);
            }
            SlotPlacement::Appended => {
                let slot = class.append_property(info);
                class.slot_names.insert(property.name.clone(), slot);
                if property.visibility == Visibility::Private {
                    class
                        .private_slots
                        .insert((link.id, property.name.clone()), slot);
                }
            }
        }

        Ok(())
    }

    fn link_methods(
        &mut self,
        class: &mut RuntimeClass,
        link: &ClassLink<'_>,
    ) -> Result<(), VirtualMachineControl> {
        for (position, method) in link.compiled.methods.iter().enumerate() {
            self.link_method(class, link, method, position)?;
        }

        Ok(())
    }

    fn link_method(
        &mut self,
        class: &mut RuntimeClass,
        link: &ClassLink<'_>,
        method: &CompiledMethod,
        position: usize,
    ) -> Result<(), VirtualMachineControl> {
        let function = FuncId(u32_index(self.tables.functions.len()));
        self.tables.functions.push(runtime_function(
            &method.function,
            link.context,
            FunctionLocator::Method {
                class: u32_index(link.class_index),
                method: u32_index(position),
            },
            Some(link.id),
        ));

        let entry = MethodEntry {
            visibility: method.visibility,
            is_static: method.is_static,
            is_abstract: method.is_abstract,
            is_final: method.is_final,
            declaring_class: link.id,
            body: MethodBodyKind::Bytecode(function),
        };

        if method.visibility == Visibility::Private {
            class
                .private_methods
                .insert((link.id, method.name.clone()), entry);
        }
        if let Some(existing) = class.members.get(&method.name)
            && !matches!(existing, ClassMemberEntry::Method(_))
        {
            return Err(self.linker_error_at(
                &link.unit.path,
                link.compiled.span,
                format!(
                    "{} declares the method {}, but already has a {} of that name",
                    link.name,
                    method.name,
                    existing.kind()
                ),
            ));
        }

        class
            .members
            .insert(method.name.clone(), ClassMemberEntry::Method(entry));
        Ok(())
    }

    fn link_enum_methods(
        &mut self,
        class: &mut RuntimeClass,
        link: &ClassLink<'_>,
    ) -> Result<(), VirtualMachineControl> {
        if link.compiled.kind != ClassLikeKind::Enum {
            return Ok(());
        }

        self.link_enum_method(class, link, b"cases", classes::enum_cases_body())?;
        if link.compiled.enum_backing.is_some() {
            self.link_enum_method(class, link, b"from", classes::enum_from_body())?;
            self.link_enum_method(class, link, b"tryFrom", classes::enum_try_from_body())?;
        }

        Ok(())
    }

    fn link_enum_method(
        &mut self,
        class: &mut RuntimeClass,
        link: &ClassLink<'_>,
        name: &[u8],
        body: BuiltInMethodBody,
    ) -> Result<(), VirtualMachineControl> {
        let name = self.heap.intern(name);
        if class.members.contains_key(&name) {
            return Err(VirtualMachineControl::Throw(self.declaration_error(
                self.tables.well_known.linker_error,
                format!(
                    "the enum {} cannot redeclare the built-in method {name}",
                    link.name
                ),
                &link.unit.path,
            )));
        }

        class.members.insert(
            name,
            ClassMemberEntry::Method(MethodEntry {
                visibility: Visibility::Public,
                is_static: true,
                is_abstract: false,
                is_final: true,
                declaring_class: link.id,
                body: MethodBodyKind::BuiltIn(body),
            }),
        );

        Ok(())
    }

    fn inherit_interface_methods(
        &mut self,
        class: &mut RuntimeClass,
        link: &ClassLink<'_>,
    ) -> Result<(), VirtualMachineControl> {
        for interface_id in link.direct_interfaces {
            let methods: Vec<(Atom, MethodEntry)> = self.tables.classes[interface_id.0 as usize]
                .methods()
                .filter(|(_, entry)| !entry.is_abstract)
                .map(|(name, entry)| (name.clone(), entry))
                .collect();

            for (name, entry) in methods {
                self.inherit_interface_method(class, link, name, entry)?;
            }
        }

        Ok(())
    }

    fn inherit_interface_method(
        &mut self,
        class: &mut RuntimeClass,
        link: &ClassLink<'_>,
        name: Atom,
        entry: MethodEntry,
    ) -> Result<(), VirtualMachineControl> {
        let Some(existing) = class.members.get(&name) else {
            class.members.insert(name, ClassMemberEntry::Method(entry));
            return Ok(());
        };

        let ClassMemberEntry::Method(existing) = existing else {
            return Err(self.linker_error_at(
                &link.unit.path,
                link.compiled.span,
                format!(
                    "{} inherits a method named {}, but already has a {} of that name",
                    link.name,
                    name,
                    existing.kind()
                ),
            ));
        };

        let same = matches!(
            (existing.body, entry.body),
            (MethodBodyKind::Bytecode(left), MethodBodyKind::Bytecode(right)) if left == right
        );

        let own = link
            .compiled
            .methods
            .iter()
            .any(|method| method.name == name)
            || (link.compiled.kind == ClassLikeKind::Enum
                && matches!(name.as_bytes(), b"cases" | b"from" | b"tryFrom"));
        let inherited = link.parent.is_some_and(|parent| {
            self.tables.classes[parent.0 as usize]
                .method(&name)
                .is_some()
        });

        if same || own || inherited {
            return Ok(());
        }

        Err(self.linker_error_at(
            &link.unit.path,
            link.compiled.span,
            format!(
                "{} receives conflicting default implementations of {name} from its interfaces",
                link.name
            ),
        ))
    }

    fn inherit_interface_constants(
        &mut self,
        class: &mut RuntimeClass,
        link: &ClassLink<'_>,
    ) -> Result<(), VirtualMachineControl> {
        for interface_id in link.direct_interfaces {
            let constants: Vec<(Atom, ClassConstantEntry)> = self.tables.classes
                [interface_id.0 as usize]
                .constants()
                .map(|(name, entry)| (name.clone(), entry.clone()))
                .collect();

            for (name, entry) in constants {
                let Some(existing) = class.members.get(&name) else {
                    class
                        .members
                        .insert(name, ClassMemberEntry::Constant(entry));
                    continue;
                };

                let ClassMemberEntry::Constant(existing) = existing else {
                    return Err(self.linker_error_at(
                        &link.unit.path,
                        link.compiled.span,
                        format!(
                            "{} inherits a constant named {}, but already has a {} of that name",
                            link.name,
                            name,
                            existing.kind()
                        ),
                    ));
                };

                if existing.declaring_class != entry.declaring_class {
                    return Err(VirtualMachineControl::Throw(self.declaration_error(
                        self.tables.well_known.linker_error,
                        format!(
                            "{} inherits conflicting declarations of the constant {} from its \
                             interfaces",
                            link.name, name
                        ),
                        &link.unit.path,
                    )));
                }
            }
        }

        Ok(())
    }

    fn link_constants(
        &mut self,
        class: &mut RuntimeClass,
        link: &ClassLink<'_>,
    ) -> Result<(), VirtualMachineControl> {
        for (position, constant) in link.compiled.constants.iter().enumerate() {
            self.link_constant(class, link, constant, position)?;
        }

        Ok(())
    }

    fn link_constant(
        &mut self,
        class: &mut RuntimeClass,
        link: &ClassLink<'_>,
        constant: &CompiledClassConstant,
        position: usize,
    ) -> Result<(), VirtualMachineControl> {
        if let Some(member) = class.members.get(&constant.name) {
            let ClassMemberEntry::Constant(existing) = member else {
                return Err(self.linker_error_at(
                    &link.unit.path,
                    link.compiled.span,
                    format!(
                        "{} declares the constant {}, but already has a {} of that name",
                        link.name,
                        constant.name,
                        member.kind()
                    ),
                ));
            };

            if existing.declaring_class == link.id {
                return Ok(());
            }

            let owner = &self.tables.classes[existing.declaring_class.0 as usize];
            let owner_kind = owner.kind;
            let owner_name = owner.name.clone();
            if owner_kind == ClassLikeKind::Interface {
                return Err(VirtualMachineControl::Throw(self.declaration_error(
                    self.tables.well_known.linker_error,
                    format!(
                        "{} redeclares the constant {}, which it inherits from the interface {}",
                        link.name, constant.name, owner_name
                    ),
                    &link.unit.path,
                )));
            }

            if existing.visibility != Visibility::Private {
                let inherited = existing
                    .declared_type
                    .clone()
                    .unwrap_or(TypeDescriptor::Mixed);
                let replacement = constant
                    .declared_type
                    .clone()
                    .unwrap_or(TypeDescriptor::Mixed);
                if !self.link_descriptor_is_subtype(&replacement, &inherited)? {
                    return Err(VirtualMachineControl::Throw(self.declaration_error(
                        self.tables.well_known.linker_error,
                        format!(
                            "the type of {}::{} is not compatible with {}::{}",
                            link.name, constant.name, owner_name, constant.name
                        ),
                        &link.unit.path,
                    )));
                }
            }
        }

        class.members.insert(
            constant.name.clone(),
            ClassMemberEntry::Constant(ClassConstantEntry {
                value: ClassConstantValue::Pending {
                    context: Rc::clone(link.context),
                    class_position: u32_index(link.class_index),
                    constant_position: u32_index(position),
                },
                declared_type: constant.declared_type.clone(),
                visibility: constant.visibility,
                declaring_class: link.id,
            }),
        );

        Ok(())
    }

    fn link_enum_members(
        &mut self,
        class: &mut RuntimeClass,
        link: &ClassLink<'_>,
    ) -> Result<(), VirtualMachineControl> {
        if link.compiled.kind != ClassLikeKind::Enum {
            return Ok(());
        }

        self.add_enum_slots(class, link);
        let mut seen_backings = EnumBackings::default();
        for case in &link.compiled.cases {
            self.link_enum_case(class, link, case, &mut seen_backings)?;
        }

        Ok(())
    }

    fn add_enum_slots(&self, class: &mut RuntimeClass, link: &ClassLink<'_>) {
        let name = self.heap.intern(b"name");
        class
            .slot_names
            .insert(name.clone(), u32_index(class.slots.len()));
        class.append_property(PropertyInfo {
            name,
            visibility: Visibility::Public,
            is_readonly: true,
            declaring_class: link.id,
            default: None,
            declared_type: Some(TypeDescriptor::String),
        });

        let Some(backing) = link.compiled.enum_backing else {
            return;
        };

        let value = self.heap.intern(b"value");
        class
            .slot_names
            .insert(value.clone(), u32_index(class.slots.len()));
        class.append_property(PropertyInfo {
            name: value,
            visibility: Visibility::Public,
            is_readonly: true,
            declaring_class: link.id,
            default: None,
            declared_type: Some(match backing {
                EnumBacking::Int => TypeDescriptor::Int,
                EnumBacking::String => TypeDescriptor::String,
            }),
        });
    }

    fn link_enum_case(
        &mut self,
        class: &mut RuntimeClass,
        link: &ClassLink<'_>,
        case: &CompiledEnumCase,
        seen_backings: &mut EnumBackings,
    ) -> Result<(), VirtualMachineControl> {
        let backing = self.evaluate_enum_backing(link, case)?;
        if let Some(value) = &backing
            && let Some(first) = seen_backings.insert(value, &case.name)
        {
            return Err(VirtualMachineControl::Throw(self.declaration_error(
                self.tables.well_known.linker_error,
                format!(
                    "the backing value of {}::{} is already used by {}::{}",
                    link.name, case.name, link.name, first
                ),
                &link.unit.path,
            )));
        }

        let position = u32_index(class.enum_cases.len());
        class.enum_cases.push(EnumCaseDeclaration {
            name: case.name.clone(),
            backing,
        });

        if let Some(existing) = class
            .members
            .insert(case.name.clone(), ClassMemberEntry::EnumCase(position))
        {
            return Err(self.linker_error_at(
                &link.unit.path,
                link.compiled.span,
                format!(
                    "{} declares the case {}, but already has a {} of that name",
                    link.name,
                    case.name,
                    existing.kind()
                ),
            ));
        }

        Ok(())
    }

    fn evaluate_enum_backing(
        &mut self,
        link: &ClassLink<'_>,
        case: &CompiledEnumCase,
    ) -> Result<Option<Value>, VirtualMachineControl> {
        let Some(initializer) = &case.value else {
            return Ok(None);
        };

        let value = self.evaluate_initializer(initializer, link.context)?;
        let Some(expected) = link.compiled.enum_backing else {
            return Ok(Some(value));
        };

        let (satisfied, expected_name) = match expected {
            EnumBacking::Int => (value.is_int(), "int"),
            EnumBacking::String => (value.is_string(), "string"),
        };

        if satisfied {
            return Ok(Some(value));
        }

        Err(VirtualMachineControl::Throw(self.declaration_error(
            self.tables.well_known.linker_error,
            format!(
                "the case {}::{} must be backed by {expected_name}, {} given",
                link.name,
                case.name,
                value.kind_name()
            ),
            &link.unit.path,
        )))
    }

    fn resolve_parent(
        &mut self,
        base: &CompiledBaseReference,
        child_text: &str,
        path: &Atom,
    ) -> Result<ClassId, VirtualMachineControl> {
        let parent_name = base.name.clone();
        let parent_text = parent_name.to_string();
        let Some(entry) = self.tables.symbols.get(&parent_name).copied() else {
            return Err(self.linker_error_at(
                path,
                base.span,
                format!("the class {parent_text} is not defined"),
            ));
        };

        if entry.kind != SymbolKind::Class {
            return Err(self.linker_error_at(
                path,
                base.span,
                format!("{child_text} cannot extend {parent_text}, which is not a class"),
            ));
        }

        let parent = &self.tables.classes[entry.index as usize];
        if parent.is_final {
            return Err(self.linker_error_at(
                path,
                base.span,
                format!("{child_text} cannot extend the final class {parent_text}"),
            ));
        }

        Ok(ClassId(entry.index))
    }

    fn resolve_interface(
        &mut self,
        base: &CompiledBaseReference,
        child_text: &str,
        path: &Atom,
    ) -> Result<ClassId, VirtualMachineControl> {
        let interface_name = base.name.clone();
        let interface_text = interface_name.to_string();
        let Some(entry) = self.tables.symbols.get(&interface_name).copied() else {
            return Err(self.linker_error_at(
                path,
                base.span,
                format!("the interface {interface_text} is not defined"),
            ));
        };

        if entry.kind != SymbolKind::Interface {
            return Err(self.linker_error_at(
                path,
                base.span,
                format!(
                    "{child_text} cannot implement {interface_text}, which is not an interface"
                ),
            ));
        }

        Ok(ClassId(entry.index))
    }
}

fn property_default_template(heap: &Heap, initializer: &ConstantInitializer) -> Option<Value> {
    let ConstantInitializer::Thunk(chunk) = initializer else {
        let ConstantInitializer::Literal(literal) = initializer else {
            return None;
        };

        return Some(literal_value(literal));
    };

    let [creation, returned] = chunk.code.as_slice() else {
        return None;
    };

    let (destination, value) = match *creation {
        Instruction::NewVec {
            element_count,
            destination,
            ..
        } if element_count.value() == 0 => (destination, Value::vec(VecObject::new(heap))),
        Instruction::NewDict {
            pair_count,
            destination,
            ..
        } if pair_count.value() == 0 => (destination, Value::dict(DictObject::new(heap))),
        _ => return None,
    };

    match *returned {
        Instruction::Return { source }
        | Instruction::ReturnUnchecked { source }
        | Instruction::ReturnReferenceUnchecked { source }
            if source == destination =>
        {
            Some(value)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::engine::Engine;
    use crate::engine::EngineConfiguration;

    #[test]
    fn dependency_linking_preserves_stable_declaration_pass_order() {
        let mut engine = Engine::new(EngineConfiguration::default());
        let outcome = engine.run_source(
            "class A extends D {}\nclass B extends C {}\nclass C {}\nclass D {}",
            Path::new("/link-order.whim"),
        );

        assert_eq!(outcome.exit_code(), 0);
        let indexes: Vec<_> = ["C", "D", "A", "B"]
            .map(|name| {
                let name = engine.heap.intern(name.as_bytes());
                engine.tables.symbols[&name].index
            })
            .into_iter()
            .collect();
        assert!(indexes.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn interface_closure_deduplicates_diamonds_and_parent_interfaces() {
        let mut engine = Engine::new(EngineConfiguration::default());
        let outcome = engine.run_source(
            "interface Root {}\n\
             interface Left extends Root {}\n\
             interface Right extends Root {}\n\
             interface Leaf extends Left, Right {}\n\
             class Parent implements Left {}\n\
             class Subject extends Parent implements Leaf, Right {}\n\
             assert!(new Subject() is Root);\n\
             assert!(new Subject() is Left);\n\
             assert!(new Subject() is Right);\n\
             assert!(new Subject() is Leaf);",
            Path::new("/interface-closure.whim"),
        );

        assert_eq!(outcome.exit_code(), 0);
        let subject = engine.heap.intern(b"Subject");
        let subject = engine.tables.symbols[&subject].index as usize;
        assert_eq!(engine.tables.classes[subject].interfaces.len(), 4);
    }
}
