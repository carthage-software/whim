//! Constructing errors, capturing stack traces, and rendering values.

use std::collections::HashSet;
use std::env::current_dir;
use std::fmt::Write;
use std::mem;
use std::rc::Rc;

use crate::builtin::spec::ParameterSpec;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::unit::BuiltInCallableAttributes;
use crate::bytecode::unit::ClassLikeKind;
use crate::bytecode::unit::TRACE_BOUNDARY_ATTRIBUTE;
use crate::bytecode::unit::TRACK_CALLER_ATTRIBUTE;
use crate::bytecode::unit::Visibility;
use crate::bytecode::unit::has_attribute;
use crate::classes::PropertyInfo;
use crate::core::classes::ERROR_SLOT_TRACE;
use crate::engine::diagnostics::DiagnosticLabel;
use crate::engine::diagnostics::DiagnosticLabels;
use crate::engine::diagnostics::DiagnosticOrigin;
use crate::path::path_from_bytes;
use crate::symbols::UnitSourceFile;
use crate::value::ValueView;
use crate::value::function::PresetArg;
use crate::vm::Atom;
use crate::vm::ByteStringObject;
use crate::vm::CallTarget;
use crate::vm::ClassId;
use crate::vm::CollectionFault;
use crate::vm::DictObject;
use crate::vm::Fault;
use crate::vm::FaultKind;
use crate::vm::Frame;
use crate::vm::FunctionObject;
use crate::vm::Heap;
use crate::vm::InstanceObject;
use crate::vm::KeyRef;
use crate::vm::Literal;
use crate::vm::ManagedRef;
use crate::vm::Register;
use crate::vm::TRACE_FRAME_SLOT_ARGUMENTS;
use crate::vm::TRACE_FRAME_SLOT_FILE;
use crate::vm::TRACE_FRAME_SLOT_FUNCTION;
use crate::vm::TRACE_FRAME_SLOT_LINE;
use crate::vm::Throw;
use crate::vm::TupleObject;
use crate::vm::UnitContext;
use crate::vm::Value;
use crate::vm::VecObject;
use crate::vm::VirtualMachine;
use crate::vm::VirtualMachineControl;
use crate::vm::line_of;
use crate::vm::ops;
use crate::vm::unreachable_invariant;

const ASSERTION_VALUE_LIMIT: usize = 512;
const DEBUG_ITEM_LIMIT: usize = 64;
const DEBUG_DEPTH_LIMIT: u32 = 32;

struct BoundedDebug {
    output: String,
    truncated: bool,
}

impl BoundedDebug {
    fn new() -> Self {
        Self {
            output: String::new(),
            truncated: false,
        }
    }

    fn push(&mut self, text: &str) {
        if self.truncated {
            return;
        }
        if self.output.len() + text.len() <= ASSERTION_VALUE_LIMIT {
            self.output.push_str(text);
            return;
        }

        let boundary = ASSERTION_VALUE_LIMIT.saturating_sub(3);
        if self.output.len() < boundary {
            let mut end = (boundary - self.output.len()).min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }

            self.output.push_str(&text[..end]);
        }

        self.mark_truncated();
    }

    fn mark_truncated(&mut self) {
        let boundary = ASSERTION_VALUE_LIMIT.saturating_sub(3);
        while self.output.len() > boundary {
            self.output.pop();
        }

        self.output.push_str("...");
        self.truncated = true;
    }

    fn push_bytes(&mut self, bytes: &[u8]) {
        if self.truncated {
            return;
        }

        let remaining = ASSERTION_VALUE_LIMIT
            .saturating_sub(3)
            .saturating_sub(self.output.len());
        let inspected = bytes.len().min(remaining.saturating_add(1));
        self.push(String::from_utf8_lossy(&bytes[..inspected]).as_ref());
        if inspected < bytes.len() && !self.truncated {
            self.mark_truncated();
        }
    }

    fn finish(self) -> String {
        self.output
    }
}

fn trace_vec(heap: &Heap, elements: impl IntoIterator<Item = Value>) -> ManagedRef<VecObject> {
    VecObject::with_elements(heap, elements)
}

pub(in crate::vm) fn visibility_name(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Public => "public",
        Visibility::Protected => "protected",
        Visibility::Private => "private",
    }
}

pub(crate) fn debug_render(heap: &Heap, value: &Value, depth: u32) -> String {
    if depth > 4 {
        return "...".to_string();
    }

    match value.transparent() {
        ValueView::Uninitialized => "uninitialized".to_string(),
        ValueView::Null => "null".to_string(),
        ValueView::Bool(value) => value.to_string(),
        ValueView::Int(rendered) => rendered.to_string(),
        ValueView::Float(rendered) => {
            String::from_utf8_lossy(ops::render_float(heap, *rendered).flatten()).into_owned()
        }
        ValueView::String(string) => {
            format!("'{}'", String::from_utf8_lossy(string.flatten()))
        }
        ValueView::ShortString(string) => {
            format!("'{}'", String::from_utf8_lossy(string.as_bytes()))
        }
        ValueView::Vec(vec) => {
            let parts: Vec<String> = vec
                .iter()
                .map(|element| debug_render(heap, element, depth + 1))
                .collect();
            format!("vec[{}]", parts.join(", "))
        }
        ValueView::Dict(dict) => {
            let parts: Vec<String> = dict
                .iter()
                .map(|(key, entry)| {
                    let key = match key {
                        KeyRef::Int(rendered) => rendered.to_string(),
                        KeyRef::Bool(rendered) => rendered.to_string(),
                        KeyRef::String(rendered) => {
                            format!("'{}'", String::from_utf8_lossy(rendered.flatten()))
                        }
                        KeyRef::ShortString(rendered) => {
                            format!("'{}'", String::from_utf8_lossy(rendered.as_bytes()))
                        }
                    };

                    format!("{key} => {}", debug_render(heap, entry, depth + 1))
                })
                .collect();
            format!("dict[{}]", parts.join(", "))
        }
        ValueView::Tuple(tuple) => {
            let parts: Vec<String> = tuple
                .iter()
                .map(|element| debug_render(heap, element, depth + 1))
                .collect();
            let trailing = if tuple.len() == 1 { "," } else { "" };
            format!("({}{trailing})", parts.join(", "))
        }
        ValueView::Function(function) => function.signature().to_string(),
        ValueView::Object(_) | ValueView::Iter(_) => value.kind_name().to_string(),
    }
}

struct DetailedDebugRenderer<'vm, 'engine> {
    vm: &'vm VirtualMachine<'engine>,
    active: HashSet<usize>,
}

impl<'vm, 'engine> DetailedDebugRenderer<'vm, 'engine> {
    fn new(vm: &'vm VirtualMachine<'engine>) -> Self {
        Self {
            vm,
            active: HashSet::new(),
        }
    }

    fn render(&mut self, value: &Value, depth: u32) -> String {
        if depth >= DEBUG_DEPTH_LIMIT {
            return format!("...<maximum depth {DEBUG_DEPTH_LIMIT} reached>");
        }

        if let Some(id) = value.newtype_id() {
            let descriptor = self.vm.engine.tables.newtype_value(id);
            let inner = value.clone_with_newtype(descriptor.parent);
            return format!(
                "{}({})",
                self.vm.runtime_type_name(value),
                self.render(&inner, depth + 1),
            );
        }

        let identity = value
            .collectable_box()
            .map(|pointer| pointer.as_ptr() as usize);
        if let Some(identity) = identity
            && !self.active.insert(identity)
        {
            return format!(
                "...<recursive reference to {}>",
                self.vm.runtime_type_name(value),
            );
        }

        let rendered = match value.transparent() {
            ValueView::Uninitialized => "uninitialized".to_string(),
            ValueView::Null => "null".to_string(),
            ValueView::Bool(value) => value.to_string(),
            ValueView::Int(value) => value.to_string(),
            ValueView::Float(value) => {
                String::from_utf8_lossy(ops::render_float(&self.vm.heap, *value).flatten())
                    .into_owned()
            }
            ValueView::String(value) => render_debug_string(value.flatten()),
            ValueView::ShortString(value) => render_debug_string(value.as_bytes()),
            ValueView::Vec(value) => self.render_vec(value, depth),
            ValueView::Dict(value) => self.render_dict(value, depth),
            ValueView::Tuple(value) => self.render_tuple(value, depth),
            ValueView::Function(value) => self.render_function(value, depth),
            ValueView::Object(value) => self.render_object(value, depth),
            ValueView::Iter(_) => self.vm.runtime_type_name(value),
        };

        if let Some(identity) = identity {
            self.active.remove(&identity);
        }

        rendered
    }

    fn render_vec(&mut self, value: &ManagedRef<VecObject>, depth: u32) -> String {
        let name = format!("vec({})", value.len());
        self.render_sequence(name, value.len(), value.iter(), depth)
    }

    fn render_dict(&mut self, value: &ManagedRef<DictObject>, depth: u32) -> String {
        let name = format!("dict({})", value.len());
        if value.is_empty() {
            return format!("{name} []");
        }

        let indent = debug_indent(depth + 1);
        let mut rendered = format!("{name} [\n");
        for (key, entry) in value.iter().take(DEBUG_ITEM_LIMIT) {
            rendered.push_str(&indent);
            rendered.push_str(&Self::render_key(key));
            rendered.push_str(" => ");
            rendered.push_str(&self.render(entry, depth + 1));
            rendered.push_str(",\n");
        }
        Self::render_redaction(value.len(), &indent, &mut rendered);
        rendered.push_str(&debug_indent(depth));
        rendered.push(']');
        rendered
    }

    fn render_tuple(&mut self, value: &ManagedRef<TupleObject>, depth: u32) -> String {
        let name = format!("tuple({})", value.len());
        self.render_sequence(name, value.len(), value.iter(), depth)
    }

    fn render_sequence<'value>(
        &mut self,
        name: String,
        length: usize,
        values: impl Iterator<Item = &'value Value>,
        depth: u32,
    ) -> String {
        if length == 0 {
            return format!("{name} []");
        }

        let indent = debug_indent(depth + 1);
        let mut rendered = format!("{name} [\n");
        for value in values.take(DEBUG_ITEM_LIMIT) {
            rendered.push_str(&indent);
            rendered.push_str(&self.render(value, depth + 1));
            rendered.push_str(",\n");
        }
        Self::render_redaction(length, &indent, &mut rendered);
        rendered.push_str(&debug_indent(depth));
        rendered.push(']');
        rendered
    }

    fn render_redaction(length: usize, indent: &str, rendered: &mut String) {
        if length <= DEBUG_ITEM_LIMIT {
            return;
        }

        rendered.push_str(indent);
        let _ = writeln!(
            rendered,
            "...<{} more items redacted>",
            length - DEBUG_ITEM_LIMIT,
        );
    }

    fn render_key(key: KeyRef<'_>) -> String {
        match key {
            KeyRef::Int(value) => value.to_string(),
            KeyRef::Bool(value) => value.to_string(),
            KeyRef::String(value) => render_debug_string(value.flatten()),
            KeyRef::ShortString(value) => render_debug_string(value.as_bytes()),
        }
    }

    fn render_function(&mut self, function: &ManagedRef<FunctionObject>, depth: u32) -> String {
        let signature = self
            .vm
            .runtime_type_name(&Value::function(function.clone()));
        let target = match function.target() {
            CallTarget::User(id) => self.vm.engine.tables.functions[id.0 as usize]
                .name
                .to_string(),
            CallTarget::BuiltIn(id) => {
                self.vm.engine.tables.built_in_functions[id.0 as usize].display_name()
            }
        };
        let indent = debug_indent(depth + 1);
        let mut rendered = format!("{signature} {{\n{indent}target = {target};");

        if let Some(this) = function.this() {
            let this = Value::object(this.clone());
            rendered.push('\n');
            rendered.push_str(&indent);
            rendered.push_str("this = ");
            rendered.push_str(&self.render(&this, depth + 1));
            rendered.push(';');
        }

        if !function.captures().is_empty() {
            let _ = write!(
                rendered,
                "\n{indent}captures({}) [\n",
                function.captures().len(),
            );
            let child_indent = debug_indent(depth + 2);
            for (index, capture) in function
                .captures()
                .iter()
                .take(DEBUG_ITEM_LIMIT)
                .enumerate()
            {
                let _ = writeln!(
                    rendered,
                    "{child_indent}{index} => {},",
                    self.render(capture, depth + 2),
                );
            }
            Self::render_redaction(function.captures().len(), &child_indent, &mut rendered);
            rendered.push_str(&indent);
            rendered.push_str("];");
        }

        if !function.presets().is_empty() {
            let _ = write!(
                rendered,
                "\n{indent}presets({}) [\n",
                function.presets().len(),
            );
            let child_indent = debug_indent(depth + 2);
            for preset in function.presets().iter().take(DEBUG_ITEM_LIMIT) {
                rendered.push_str(&child_indent);
                match preset {
                    PresetArg::Given(value) => {
                        rendered.push_str(&self.render(value, depth + 2));
                    }
                    PresetArg::Hole(position) => {
                        let _ = write!(rendered, "<hole {position}>");
                    }
                }
                rendered.push_str(",\n");
            }
            Self::render_redaction(function.presets().len(), &child_indent, &mut rendered);
            rendered.push_str(&indent);
            rendered.push_str("];");
        }

        rendered.push('\n');
        rendered.push_str(&debug_indent(depth));
        rendered.push('}');
        rendered
    }

    fn render_object(&mut self, instance: &ManagedRef<InstanceObject>, depth: u32) -> String {
        if instance.class() == self.vm.engine.tables.whim_classes.sensitive_parameter_value {
            return "Whim\\Marker\\SensitiveParameterValue(<redacted>)".to_string();
        }

        if let Some(rendered) = self.render_enum_case(instance, depth) {
            return rendered;
        }

        let value = Value::object(instance.clone());
        let name = self.vm.runtime_type_name(&value);
        let class = &self.vm.engine.tables.classes[instance.class().0 as usize];
        if class.slots.is_empty() {
            return format!("{name} {{}}");
        }

        let indent = debug_indent(depth + 1);
        let mut rendered = format!("{name} {{\n");
        for (index, property) in class.slots.iter().enumerate() {
            rendered.push_str(&indent);
            rendered.push_str(visibility_name(property.visibility));
            rendered.push(' ');
            if property.is_readonly {
                rendered.push_str("readonly ");
            }
            rendered.push_str(&self.property_type(instance, property));
            rendered.push_str(" $");
            rendered.push_str(&property.name.to_string_lossy());
            rendered.push_str(" = ");
            if property.visibility == Visibility::Public {
                rendered.push_str(&self.render(&instance.read_slot(index), depth + 1));
            } else {
                rendered.push_str("<hidden>");
            }
            rendered.push_str(";\n");
        }
        rendered.push_str(&debug_indent(depth));
        rendered.push('}');
        rendered
    }

    fn render_enum_case(
        &mut self,
        instance: &ManagedRef<InstanceObject>,
        depth: u32,
    ) -> Option<String> {
        let class = &self.vm.engine.tables.classes[instance.class().0 as usize];
        if class.kind != ClassLikeKind::Enum {
            return None;
        }

        let name_slot = class.slot_names.get(&self.vm.engine.heap.intern(b"name"))?;
        let name = instance.read_slot(*name_slot as usize);
        let name = name.as_string_bytes()?;
        let mut rendered = format!(
            "{}::{}",
            self.vm.runtime_type_name(&Value::object(instance.clone())),
            String::from_utf8_lossy(name),
        );
        if let Some(value_slot) = class.slot_names.get(&self.vm.engine.heap.intern(b"value")) {
            rendered.push('(');
            rendered.push_str(&self.render(&instance.read_slot(*value_slot as usize), depth + 1));
            rendered.push(')');
        }
        Some(rendered)
    }

    fn property_type(
        &self,
        instance: &ManagedRef<InstanceObject>,
        property: &PropertyInfo,
    ) -> String {
        let Some(declared) = &property.declared_type else {
            return "mixed".to_string();
        };
        if property.declaring_class == instance.class() {
            let descriptor =
                self.vm
                    .substitute_descriptor(declared, instance.type_environment(), 0);
            return self.vm.render_descriptor(&descriptor);
        }

        let class = &self.vm.engine.tables.classes[instance.class().0 as usize];
        let declaring = &self.vm.engine.tables.classes[property.declaring_class.0 as usize];
        let Some(arguments) = class.base_specializations.get(&property.declaring_class) else {
            return self.vm.render_descriptor(declared);
        };
        let bindings: Vec<(Atom, TypeDescriptor)> = declaring
            .type_parameters
            .iter()
            .zip(arguments)
            .map(|(parameter, argument)| {
                (
                    parameter.name.clone(),
                    self.vm
                        .substitute_descriptor(argument, instance.type_environment(), 0),
                )
            })
            .collect();
        self.vm
            .render_descriptor(&substitute_debug_parameters(declared, &bindings))
    }
}

fn substitute_debug_parameters(
    descriptor: &TypeDescriptor,
    bindings: &[(Atom, TypeDescriptor)],
) -> TypeDescriptor {
    if let TypeDescriptor::Parameter(name) = descriptor
        && let Some((_, value)) = bindings.iter().rev().find(|(key, _)| key == name)
    {
        return value.clone();
    }

    descriptor.map_children(|child| substitute_debug_parameters(child, bindings))
}

fn debug_indent(depth: u32) -> String {
    "  ".repeat(depth as usize)
}

fn render_debug_string(bytes: &[u8]) -> String {
    let mut escaped = String::new();
    let mut remaining = bytes;
    while !remaining.is_empty() {
        match str::from_utf8(remaining) {
            Ok(valid) => {
                render_debug_characters(valid, &mut escaped);
                break;
            }
            Err(error) => {
                let valid_length = error.valid_up_to();
                if valid_length != 0 {
                    let valid = String::from_utf8_lossy(&remaining[..valid_length]);
                    render_debug_characters(&valid, &mut escaped);
                }
                let invalid_length = error.error_len().unwrap_or(1);
                for byte in &remaining[valid_length..valid_length + invalid_length] {
                    let _ = write!(escaped, "\\x{byte:02x}");
                }
                remaining = &remaining[valid_length + invalid_length..];
            }
        }
    }

    format!("string({}) \"{escaped}\"", bytes.len())
}

fn render_debug_characters(value: &str, rendered: &mut String) {
    for character in value.chars() {
        match character {
            '"' => rendered.push_str("\\\""),
            '\\' => rendered.push_str("\\\\"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            '\0' => rendered.push_str("\\x00"),
            character if character.is_control() => rendered.extend(character.escape_unicode()),
            character => rendered.push(character),
        }
    }
}

impl VirtualMachine<'_> {
    fn debug_render_enum_case(
        &self,
        instance: &ManagedRef<InstanceObject>,
        depth: u32,
    ) -> Option<String> {
        let class = &self.engine.tables.classes[instance.class().0 as usize];
        if class.kind != ClassLikeKind::Enum {
            return None;
        }

        let name_slot = class.slot_names.get(&self.engine.heap.intern(b"name"))?;
        let name = instance.read_slot(*name_slot as usize);
        let name = name.as_string_bytes()?;
        let mut rendered = format!(
            "{}::{}",
            self.runtime_type_name(&Value::object(instance.clone())),
            String::from_utf8_lossy(name),
        );

        if let Some(value_slot) = class.slot_names.get(&self.engine.heap.intern(b"value")) {
            rendered.push('(');
            rendered.push_str(&debug_render(
                &self.heap,
                &instance.read_slot(*value_slot as usize),
                depth + 1,
            ));
            rendered.push(')');
        }

        Some(rendered)
    }

    pub(in crate::vm) fn debug_render_value(&self, value: &Value) -> String {
        DetailedDebugRenderer::new(self).render(value, 0)
    }

    /// The class-aware debug rendering used by a failed assertion. Unlike
    /// `debug!`, this stops traversing a collection once the diagnostic has
    /// reached its fixed byte budget.
    pub(in crate::vm) fn assertion_debug_render(&self, value: &Value) -> String {
        let mut rendered = BoundedDebug::new();
        self.assertion_debug_render_into(value, 0, &mut rendered);
        rendered.finish()
    }

    fn assertion_debug_render_into(&self, value: &Value, depth: u32, rendered: &mut BoundedDebug) {
        if rendered.truncated {
            return;
        }

        if depth > 4 {
            rendered.push("...");
            return;
        }

        if value.newtype_id().is_some() {
            rendered.push(&self.runtime_type_name(value));
            return;
        }

        match value.transparent() {
            ValueView::Uninitialized => rendered.push("uninitialized"),
            ValueView::Null => rendered.push("null"),
            ValueView::Bool(value) => rendered.push(if *value { "true" } else { "false" }),
            ValueView::Int(value) => rendered.push(&value.to_string()),
            ValueView::Float(value) => {
                rendered.push_bytes(ops::render_float(&self.heap, *value).flatten());
            }
            ValueView::String(value) => {
                rendered.push("'");
                rendered.push_bytes(value.flatten());
                rendered.push("'");
            }
            ValueView::ShortString(value) => {
                rendered.push("'");
                rendered.push_bytes(value.as_bytes());
                rendered.push("'");
            }
            ValueView::Vec(vec) => {
                rendered.push("vec[");
                for (index, element) in vec.iter().enumerate() {
                    if index != 0 {
                        rendered.push(", ");
                    }

                    self.assertion_debug_render_into(element, depth + 1, rendered);
                    if rendered.truncated {
                        return;
                    }
                }

                rendered.push("]");
            }
            ValueView::Dict(dict) => {
                rendered.push("dict[");
                for (index, (key, entry)) in dict.iter().enumerate() {
                    if index != 0 {
                        rendered.push(", ");
                    }

                    match key {
                        KeyRef::Int(value) => rendered.push(&value.to_string()),
                        KeyRef::Bool(value) => rendered.push(if value { "true" } else { "false" }),
                        KeyRef::String(value) => {
                            rendered.push("'");
                            rendered.push_bytes(value.flatten());
                            rendered.push("'");
                        }
                        KeyRef::ShortString(value) => {
                            rendered.push("'");
                            rendered.push_bytes(value.as_bytes());
                            rendered.push("'");
                        }
                    }

                    rendered.push(" => ");
                    self.assertion_debug_render_into(entry, depth + 1, rendered);
                    if rendered.truncated {
                        return;
                    }
                }

                rendered.push("]");
            }
            ValueView::Tuple(tuple) => {
                rendered.push("(");
                for (index, element) in tuple.iter().enumerate() {
                    if index != 0 {
                        rendered.push(", ");
                    }

                    self.assertion_debug_render_into(element, depth + 1, rendered);
                    if rendered.truncated {
                        return;
                    }
                }

                if tuple.len() == 1 {
                    rendered.push(",");
                }

                rendered.push(")");
            }
            ValueView::Function(_) => rendered.push(&self.runtime_type_name(value)),
            ValueView::Object(instance) => {
                if let Some(value) = self.debug_render_enum_case(instance, depth) {
                    rendered.push(&value);
                } else {
                    rendered.push(&self.runtime_type_name(value));
                }
            }
            ValueView::Iter(_) => rendered.push(value.kind_name()),
        }
    }
}

/// The text of a string constant, for diagnostics.
pub(in crate::vm) fn literal_text(literal: &Literal) -> String {
    match literal {
        Literal::String(atom) => atom.to_string_lossy().into_owned(),
        // SAFETY: the surrounding invariant makes this path unreachable.
        _ => unsafe { unreachable_invariant("the diagnostic constant is a string") },
    }
}

impl VirtualMachine<'_> {
    #[cold]
    #[inline(never)]
    pub(in crate::vm) fn uninitialized_property_error(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        name: &Atom,
    ) -> VirtualMachineControl {
        let class_name = self.value_type_name(&Value::object(receiver.clone()));
        let member = name.to_string_lossy();

        self.throw_well_known(
            self.engine.tables.well_known.uninitialized_property_error,
            format!("the property {class_name}::{member} is read before initialization"),
        )
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn uninitialized_property_slot_error(
        &mut self,
        receiver: &ManagedRef<InstanceObject>,
        slot: usize,
    ) -> VirtualMachineControl {
        let name = self.engine.tables.classes[receiver.class().0 as usize].slots[slot]
            .name
            .clone();

        self.uninitialized_property_error(receiver, &name)
    }

    /// The control transfer of a failed binary operation.
    pub(in crate::vm) fn binary_fault(
        &mut self,
        fault: Fault,
        operator: &str,
        left: &'static str,
        right: &'static str,
    ) -> VirtualMachineControl {
        let (class, message) = match fault {
            Fault::Incompatible => (
                self.engine.tables.well_known.incompatible_operands_error,
                format!("the {operator} operator is not defined for {left} and {right}"),
            ),
            Fault::Unordered => (
                self.engine.tables.well_known.incompatible_operands_error,
                format!("the {operator} operator cannot order NAN"),
            ),
            Fault::DivisionByZero => (
                self.engine.tables.well_known.division_by_zero_error,
                "division by zero".to_string(),
            ),
            Fault::Overflow => (
                self.engine.tables.well_known.overflow_error,
                "the integer result overflows the 64-bit range".to_string(),
            ),
            Fault::Underflow => (
                self.engine.tables.well_known.underflow_error,
                "the integer result underflows the 64-bit range".to_string(),
            ),
            Fault::ShiftRange => (
                self.engine.tables.well_known.arithmetic_error,
                "a shift count must be between 0 and 63".to_string(),
            ),
        };
        self.throw_well_known(class, message)
    }

    /// The control transfer of a failed unary operation.
    pub(in crate::vm) fn unary_fault(
        &mut self,
        fault: Fault,
        operator: &str,
        operand: &'static str,
    ) -> VirtualMachineControl {
        match fault {
            Fault::Incompatible => {
                let class = self.engine.tables.well_known.incompatible_operands_error;
                self.throw_well_known(
                    class,
                    format!("the unary {operator} operator is not defined for {operand}"),
                )
            }
            other => self.binary_fault(other, operator, operand, operand),
        }
    }
}

impl VirtualMachine<'_> {
    /// Maps a collection fault to its error class.
    pub(in crate::vm) fn collection_fault(
        &mut self,
        fault: CollectionFault,
    ) -> VirtualMachineControl {
        let class = match fault.kind {
            FaultKind::TypeError => self.engine.tables.well_known.type_error,
            FaultKind::OutOfBounds => self.engine.tables.well_known.out_of_bounds_error,
        };
        self.throw_well_known(class, fault.message)
    }
    pub(crate) fn throw_well_known(
        &mut self,
        class: ClassId,
        message: String,
    ) -> VirtualMachineControl {
        VirtualMachineControl::Throw(self.build_error(class, message, 0))
    }
    pub(in crate::vm) fn throw_well_known_with_previous(
        &mut self,
        class: ClassId,
        message: String,
        previous: Value,
    ) -> VirtualMachineControl {
        VirtualMachineControl::Throw(self.build_error_with_previous(class, message, 0, previous))
    }
    /// Builds a well-known error as a handler [`Throw`].
    pub(crate) fn throw_well_known_value(&mut self, class: ClassId, message: String) -> Throw {
        Throw(self.build_error(class, message, 0))
    }
    /// Builds an error instance directly: slots are written by property name
    /// through the resolved layout, without running the constructor, so an
    /// engine throw is never interceptable mid-flight by a user override and
    /// can never recurse into another engine throw.
    pub(in crate::vm) fn build_error(
        &mut self,
        class: ClassId,
        message: String,
        code: i64,
    ) -> Value {
        self.build_error_with_previous(class, message, code, Value::null())
    }
    fn build_error_with_previous(
        &mut self,
        class: ClassId,
        message: String,
        code: i64,
        previous: Value,
    ) -> Value {
        let slot_count = self.engine.tables.classes[class.0 as usize].slots.len();
        let instance = InstanceObject::new(&self.heap, class, slot_count);
        let (file, line) = self.current_location();
        let trace = self.capture_trace();
        let engine = &*self.engine;
        engine.write_error_slot(
            &instance,
            class,
            b"message",
            Value::string(ByteStringObject::from_bytes(&self.heap, message.as_bytes())),
        );
        engine.write_error_slot(&instance, class, b"code", Value::int(code));
        engine.write_error_slot(&instance, class, b"previous", previous);
        engine.write_error_slot(&instance, class, b"file", file);
        engine.write_error_slot(&instance, class, b"line", line);
        engine.write_error_slot(&instance, class, b"trace", trace);
        let error = Value::object(instance);
        self.record_current_exception_origin(&error, "the exception originated here");
        error
    }
    /// The construction site of an error: the active frame's file and line,
    /// as a `(string, int)` pair. Never re-enters the interpreter.
    pub(crate) fn current_location(&self) -> (Value, Value) {
        let Some(frame) = self.frames.last() else {
            return (
                Value::string(ByteStringObject::from_bytes(&self.heap, b"")),
                Value::int(0),
            );
        };
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let unit = unsafe { frame.unit.as_ref() };
        let Some(offset) = Self::frame_offset(frame) else {
            return (Value::string(unit.path.to_handle()), Value::int(0));
        };
        let (path, line) = Self::source_location(unit, offset);
        (Value::string(path.to_handle()), Value::int(i64::from(line)))
    }

    pub(crate) fn debug_location(&self, next_instruction: usize) -> String {
        let frame = self.current_frame();
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let unit = unsafe { frame.unit.as_ref() };
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let chunk = unsafe { frame.chunk.as_ref() };
        let Some(instruction) = next_instruction.checked_sub(1) else {
            return format!("[{}:0:0] ", unit.path);
        };
        let Some(span) = chunk.spans.get(instruction) else {
            return format!("[{}:0:0] ", unit.path);
        };
        let (path, line, column) = Self::source_position(unit, span.start.offset);
        let path = path_from_bytes(path.as_bytes());
        let path = match current_dir() {
            Ok(directory) => match path.strip_prefix(directory) {
                Ok(relative) => relative,
                Err(_) => &path,
            },
            Err(_) => &path,
        };
        format!("[{}:{line}:{column}] ", path.display())
    }

    fn diagnostic_origin_for_frame(
        &self,
        frame_index: usize,
        message: &str,
    ) -> Option<DiagnosticOrigin> {
        let frame = self.frames.get(frame_index)?;
        if frame.ip == 0 {
            return None;
        }
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let chunk = unsafe { frame.chunk.as_ref() };
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let unit = unsafe { frame.unit.as_ref() };
        let source = unit.source.as_ref()?;
        let mut span = *chunk.spans.get(frame.ip as usize - 1)?;
        let (path, source) = match Self::source_file(unit, span.start.offset) {
            Some(file) => {
                let start = file.start as usize;
                let end = file.end as usize;
                span.start = span.start.backward(file.start);
                span.end = span.end.backward(file.start);
                (file.path.clone(), Rc::<str>::from(&source[start..end]))
            }
            None => (unit.path.clone(), source.to_rc()),
        };
        Some(DiagnosticOrigin {
            path,
            source,
            labels: DiagnosticLabels::Single(DiagnosticLabel {
                span,
                message: message.to_string(),
            }),
        })
    }

    fn current_diagnostic_origin(&self, message: &str) -> Option<DiagnosticOrigin> {
        self.diagnostic_origin_for_frame(self.frames.len().checked_sub(1)?, message)
    }

    /// Replaces a throwable's diagnostic origin with the active instruction.
    pub(in crate::vm) fn record_current_exception_origin(&mut self, value: &Value, message: &str) {
        let Some(origin) = self.current_diagnostic_origin(message) else {
            return;
        };
        self.engine.record_exception_origin(value, origin);
    }

    pub(in crate::vm) fn frame_tracks_caller(&self, frame_index: usize) -> bool {
        self.frames
            .get(frame_index)
            .and_then(|frame| frame.function.get())
            .is_some_and(|function| {
                has_attribute(
                    self.engine.tables.functions[function.0 as usize].attributes(),
                    TRACK_CALLER_ATTRIBUTE,
                )
            })
    }

    /// Records an explicit throw at its ordinary instruction, or at the
    /// outermost unmarked caller when the throwing call chain opts into
    /// `Whim\Marker\TrackCaller`.
    pub(in crate::vm) fn record_explicit_throw_origin(&mut self, value: &Value) {
        let Some(active) = self.frames.len().checked_sub(1) else {
            return;
        };
        if !self.frame_tracks_caller(active) {
            self.record_current_exception_origin(value, "the exception was thrown here");
            return;
        }

        let mut caller = active;
        while caller > 0 && self.frame_tracks_caller(caller) {
            caller -= 1;
        }
        let Some(origin) =
            self.diagnostic_origin_for_frame(caller, "the exception was thrown here")
        else {
            self.record_current_exception_origin(value, "the exception was thrown here");
            return;
        };

        let frame = &self.frames[active];
        let function = frame.function.get().map_or_else(
            || "{main}".to_string(),
            |function| {
                self.engine.tables.functions[function.0 as usize]
                    .name
                    .to_string()
            },
        );
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let unit = unsafe { frame.unit.as_ref() };
        let (path, line) = match Self::frame_offset(frame) {
            Some(offset) => Self::source_location(unit, offset),
            None => (&unit.path, 0),
        };
        let note = format!(
            "raised inside {function} ({}:{line})",
            path.to_string_lossy()
        );
        self.engine
            .record_exception_origin_with_note(value, origin, Some(note));
    }

    /// Relocates an engine or built-in failure through a source callable marked
    /// `TrackCaller`. Explicit throws already carry a relocation note and are
    /// left alone, so consecutive marked frames do not overwrite the original
    /// implementation location while unwinding.
    pub(in crate::vm) fn relocate_tracked_error_origin(&mut self, value: &Value, active: usize) {
        if !self.frame_tracks_caller(active) {
            return;
        }
        let frame = &self.frames[active];
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let chunk = unsafe { frame.chunk.as_ref() };
        if frame.ip != 0
            && matches!(
                chunk.code.get(frame.ip as usize - 1),
                Some(Instruction::Return { .. } | Instruction::ReturnNull)
            )
        {
            return;
        }
        if value
            .as_object()
            .and_then(|instance| self.engine.exception_note(instance))
            .is_some()
        {
            return;
        }
        let Some(instance) = value.as_object() else {
            return;
        };
        let Some(existing) = self.engine.exception_origin(instance) else {
            return;
        };
        let Some(active_origin) = self.diagnostic_origin_for_frame(active, "") else {
            return;
        };
        let existing_span = match &existing.labels {
            DiagnosticLabels::Single(label) => Some(label.span),
            DiagnosticLabels::Multiple(labels) => labels.first().map(|label| label.span),
        };
        let active_span = match &active_origin.labels {
            DiagnosticLabels::Single(label) => Some(label.span),
            DiagnosticLabels::Multiple(labels) => labels.first().map(|label| label.span),
        };
        if existing.path != active_origin.path || existing_span != active_span {
            return;
        }

        let mut caller = active;
        while caller > 0 && self.frame_tracks_caller(caller) {
            caller -= 1;
        }
        let Some(origin) =
            self.diagnostic_origin_for_frame(caller, "the exception originated here")
        else {
            return;
        };

        let function = frame.function.get().map_or_else(
            || "{main}".to_string(),
            |function| {
                self.engine.tables.functions[function.0 as usize]
                    .name
                    .to_string()
            },
        );
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let unit = unsafe { frame.unit.as_ref() };
        let (path, line) = match Self::frame_offset(frame) {
            Some(offset) => Self::source_location(unit, offset),
            None => (&unit.path, 0),
        };
        let note = format!(
            "raised inside {function} ({}:{line})",
            path.to_string_lossy()
        );
        self.engine
            .record_exception_origin_with_note(value, origin, Some(note));
    }

    /// Adds the built-in boundary that raised `value` to its already captured
    /// trace. Built-in calls do not push interpreter frames; materializing this
    /// entry only on failure keeps successful built-in calls allocation-free.
    pub(in crate::vm) fn record_built_in_trace_frame(
        &self,
        value: &Value,
        function: &str,
        parameters: &[ParameterSpec],
        arguments: &[Value],
        attributes: BuiltInCallableAttributes,
    ) {
        if attributes.trace_boundary && !self.engine.configuration.full_trace {
            return;
        }
        let Some(error) = value.as_object() else {
            return;
        };
        if !self.engine.is_throwable_instance(error.class()) {
            return;
        }

        let trace = error.read_slot(ERROR_SLOT_TRACE);
        let Some(trace) = trace.as_vec() else {
            return;
        };
        let mut entries = trace.iter().cloned().collect::<Vec<_>>();
        let visible_interpreter_frames = self
            .frames
            .iter()
            .filter(|frame| {
                self.engine.configuration.full_trace
                    || !frame.function.get().is_some_and(|function| {
                        has_attribute(
                            self.engine.tables.functions[function.0 as usize].attributes(),
                            TRACE_BOUNDARY_ATTRIBUTE,
                        )
                    })
            })
            .count();
        let insertion = entries.len().saturating_sub(visible_interpreter_frames);

        let sensitive_value_class = self.engine.tables.whim_classes.sensitive_parameter_value;
        let mut rendered_arguments = Vec::with_capacity(arguments.len().min(parameters.len()));
        for (position, argument) in arguments.iter().take(parameters.len()).enumerate() {
            let mut argument = argument.clone();
            if parameters[position].sensitive {
                let wrapper = InstanceObject::new(&self.heap, sensitive_value_class, 1);
                drop(wrapper.write_slot(0, argument));
                argument = Value::object(wrapper);
            }
            rendered_arguments.push(argument);
        }

        let trace_class = self.engine.tables.well_known.trace_frame;
        let slot_count = self.engine.tables.classes[trace_class.0 as usize]
            .slots
            .len();
        let frame = InstanceObject::new(&self.heap, trace_class, slot_count);
        drop(frame.write_slot(
            TRACE_FRAME_SLOT_FUNCTION,
            Value::string(ByteStringObject::from_bytes(
                &self.heap,
                function.as_bytes(),
            )),
        ));
        drop(frame.write_slot(
            TRACE_FRAME_SLOT_FILE,
            Value::string(ByteStringObject::from_bytes(&self.heap, b"<internal>")),
        ));
        drop(frame.write_slot(TRACE_FRAME_SLOT_LINE, Value::int(0)));
        drop(frame.write_slot(
            TRACE_FRAME_SLOT_ARGUMENTS,
            Value::vec(trace_vec(&self.heap, rendered_arguments)),
        ));
        entries.insert(insertion, Value::object(frame));
        self.engine.write_error_slot(
            error,
            error.class(),
            b"trace",
            Value::vec(trace_vec(&self.heap, entries)),
        );
    }

    pub(crate) fn throw_well_known_at(
        &mut self,
        class: ClassId,
        message: String,
        origin: DiagnosticOrigin,
    ) -> VirtualMachineControl {
        let value = self.build_error(class, message, 0);
        self.engine.record_exception_origin(&value, origin);
        VirtualMachineControl::Throw(value)
    }
    fn frame_offset(frame: &Frame) -> Option<u32> {
        if frame.ip == 0 {
            return None;
        }
        // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
        let chunk = unsafe { frame.chunk.as_ref() };
        chunk
            .spans
            .get(frame.ip as usize - 1)
            .map(|span| span.start.offset)
    }

    fn source_file(unit: &UnitContext, offset: u32) -> Option<&UnitSourceFile> {
        unit.source_files
            .iter()
            .find(|file| offset >= file.start && offset < file.end)
    }

    fn source_location(unit: &UnitContext, offset: u32) -> (&Atom, u32) {
        let (path, line, _) = Self::source_position(unit, offset);
        (path, line)
    }

    fn source_position(unit: &UnitContext, offset: u32) -> (&Atom, u32, u32) {
        let (path, line_starts, offset) = match Self::source_file(unit, offset) {
            Some(file) => (&file.path, &file.line_starts, offset - file.start),
            None => (&unit.path, &unit.line_starts, offset),
        };
        let line = line_of(line_starts, offset);
        let column = line
            .checked_sub(1)
            .and_then(|index| line_starts.get(index as usize))
            .map_or(0, |start| offset.saturating_sub(*start) + 1);
        (path, line, column)
    }
    /// Captures inner frames first and wraps sensitive arguments.
    pub(crate) fn capture_trace(&self) -> Value {
        let trace_class = self.engine.tables.well_known.trace_frame;
        let slot_count = self.engine.tables.classes[trace_class.0 as usize]
            .slots
            .len();
        let mut entries = Vec::with_capacity(self.frames.len());
        for frame_index in (0..self.frames.len()).rev() {
            let frame = &self.frames[frame_index];
            if !self.engine.configuration.full_trace
                && frame.function.get().is_some_and(|function| {
                    has_attribute(
                        self.engine.tables.functions[function.0 as usize].attributes(),
                        TRACE_BOUNDARY_ATTRIBUTE,
                    )
                })
            {
                continue;
            }
            // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
            let unit = unsafe { frame.unit.as_ref() };
            let (path, line) = match Self::frame_offset(frame) {
                Some(offset) => Self::source_location(unit, offset),
                None => (&unit.path, 0),
            };
            let first = frame.base as usize + usize::from(frame.has_this());
            let sensitive_value_class = self.engine.tables.whim_classes.sensitive_parameter_value;
            let argc = frame.argc as usize;
            let declared = frame.function.get().map_or(argc, |function| {
                usize::from(self.engine.tables.functions[function.0 as usize].declared_parameters)
            });
            let mut arguments = Vec::with_capacity(argc.min(declared));
            for position in 0..argc.min(declared) {
                // SAFETY: verified bytecode and VM state prove the index, type, and lifetime.
                let trace_register = unsafe { frame.chunk.as_ref() }
                    .trace_argument_registers
                    .get(position)
                    .copied()
                    .unwrap_or(Register::NONE);
                let source = if trace_register == Register::NONE {
                    first + position
                } else {
                    frame.base as usize + usize::from(trace_register.index())
                };
                arguments.push(self.stack[source].clone());
            }
            for (position, argument) in arguments.iter_mut().enumerate() {
                if argument.is_uninitialized() {
                    *argument = Value::null();
                }
                let sensitive = frame.function.get().is_some_and(|function| {
                    self.engine.tables.functions[function.0 as usize]
                        .parameters()
                        .get(position)
                        .is_some_and(|parameter| parameter.sensitive)
                });
                if sensitive {
                    let wrapper = InstanceObject::new(&self.heap, sensitive_value_class, 1);
                    drop(wrapper.write_slot(0, mem::replace(argument, Value::null())));
                    *argument = Value::object(wrapper);
                }
            }
            let function_name = match frame.function.get() {
                Some(function) => self.engine.tables.functions[function.0 as usize]
                    .name
                    .to_handle(),
                None => self.heap.intern(b"{main}").to_handle(),
            };
            let instance = InstanceObject::new(&self.heap, trace_class, slot_count);
            drop(instance.write_slot(TRACE_FRAME_SLOT_FUNCTION, Value::string(function_name)));
            drop(instance.write_slot(TRACE_FRAME_SLOT_FILE, Value::string(path.to_handle())));
            drop(instance.write_slot(TRACE_FRAME_SLOT_LINE, Value::int(i64::from(line))));
            drop(instance.write_slot(
                TRACE_FRAME_SLOT_ARGUMENTS,
                Value::vec(trace_vec(&self.heap, arguments)),
            ));
            entries.push(Value::object(instance));
        }
        Value::vec(trace_vec(&self.heap, entries))
    }
    /// Converts an internal control into a handler-facing [`Throw`],
    /// latching an exit for the enclosing call site to convert back.
    pub(crate) fn control_to_throw(&mut self, control: VirtualMachineControl) -> Throw {
        match control {
            VirtualMachineControl::Throw(value) => Throw(value),
            VirtualMachineControl::Exit(code) => {
                self.pending_exit = Some(code);
                Throw(Value::uninitialized())
            }
        }
    }
}
