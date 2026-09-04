//! The member set of a class-like declaration: constants, properties,
//! methods, and enum cases.

use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::cst::atom::Modifier;
use whim_syn::cst::class::ClassLikeConstant;
use whim_syn::cst::class::ClassLikeMember;
use whim_syn::cst::class::EnumCase;
use whim_syn::cst::class::Method;
use whim_syn::cst::class::MethodBody;
use whim_syn::cst::class::Property;
use whim_syn::cst::r#type::Type;

use crate::bytecode::chunk::Chunk;
use crate::bytecode::chunk::descriptors::Literal as BytecodeLiteral;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::chunk::descriptors::string_length_matches;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::unit::CompiledAttribute;
use crate::bytecode::unit::CompiledClassConstant;
use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::unit::CompiledEnumCase;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::CompiledMethod;
use crate::bytecode::unit::CompiledParameter;
use crate::bytecode::unit::CompiledProperty;
use crate::bytecode::unit::CompiledTypeAlias;
use crate::bytecode::unit::CompiledTypeParameter;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::unit::ConstantInitializer;
use crate::bytecode::unit::EnumBacking;
use crate::bytecode::unit::STUB_ATTRIBUTE;
use crate::bytecode::unit::Visibility;
use crate::compiler::declarations::class_likes::validate_variance_use;
use crate::compiler::declarations::functions::DeclarationContext;
use crate::compiler::declarations::functions::compile_attributes;
use crate::compiler::declarations::functions::compile_body;
use crate::compiler::declarations::functions::compile_initializer;
use crate::compiler::declarations::functions::compile_parameters;
use crate::compiler::declarations::functions::inert_declaration_chunk;
use crate::compiler::declarations::functions::render_signature;
use crate::compiler::declarations::generics::binder_names;
use crate::compiler::declarations::generics::compile_type_parameters;
use crate::compiler::emit::BodyShape;
use crate::compiler::emit::ReturnKind;
use crate::compiler::emit::Scope;
use crate::compiler::error::CompileError;
use crate::compiler::error::CompileErrorKind;
use crate::compiler::rules;
use crate::compiler::types;
use crate::compiler::types::ClassContext;
use crate::compiler::types::GenericTable;
use crate::compiler::types::TypeScope;
use crate::compiler::types::lowering::lower_type;
use crate::compiler::types::rendering::render_type;
use crate::limits::MAX_TYPE_DEPTH;
use crate::value::atom::Atom;

struct MethodMetadata {
    parameters: Vec<CompiledParameter>,
    attributes: Vec<CompiledAttribute>,
    type_parameters: Vec<CompiledTypeParameter>,
    return_type: Option<TypeDescriptor>,
    signature: String,
    return_kind: ReturnKind,
}

struct MemberCompiler<'compiler, 'scope> {
    scope: &'compiler Scope<'scope>,
    class_context: &'compiler ClassContext,
    path: &'compiler str,
    source_text: &'compiler str,
    unit: &'compiler mut CompiledUnit,
    output: &'compiler mut CompiledClassLike,
    class_parameters: Vec<CompiledTypeParameter>,
    class_is_readonly: bool,
    is_external: bool,
}

fn member_visibility(modifiers: &[Modifier<'_>], span: Span) -> Result<Visibility, CompileError> {
    for modifier in modifiers {
        match modifier {
            Modifier::Public(_) => return Ok(Visibility::Public),
            Modifier::Protected(_) => return Ok(Visibility::Protected),
            Modifier::Private(_) => return Ok(Visibility::Private),
            _ => {}
        }
    }

    Err(CompileError::new(
        CompileErrorKind::MemberWithoutVisibility,
        "every class-like member declares its visibility",
        span,
    ))
}

fn validate_parameter_variance(
    parameters: &[CompiledParameter],
    type_parameters: &[CompiledTypeParameter],
    generics: &GenericTable<'_>,
    span: Span,
) -> Result<(), CompileError> {
    for parameter in parameters {
        if let Some(descriptor) = &parameter.declared_type {
            validate_variance_use(descriptor, -1, type_parameters, generics, span)?;
        }
    }

    Ok(())
}

pub(in crate::compiler::declarations) fn compile_members(
    scope: &Scope<'_>,
    class_context: &ClassContext,
    members: &[ClassLikeMember<'_>],
    path: &str,
    source_text: &str,
    unit: &mut CompiledUnit,
    output: &mut CompiledClassLike,
) -> Result<(), CompileError> {
    MemberCompiler::new(scope, class_context, path, source_text, unit, output).compile(members)
}

impl<'compiler, 'scope> MemberCompiler<'compiler, 'scope> {
    fn new(
        scope: &'compiler Scope<'scope>,
        class_context: &'compiler ClassContext,
        path: &'compiler str,
        source_text: &'compiler str,
        unit: &'compiler mut CompiledUnit,
        output: &'compiler mut CompiledClassLike,
    ) -> Self {
        let class_parameters = output.type_parameters.clone();
        let class_is_readonly = output.is_readonly;
        let is_external = output
            .attributes
            .iter()
            .any(|attribute| attribute.class.as_bytes() == STUB_ATTRIBUTE);
        Self {
            scope,
            class_context,
            path,
            source_text,
            unit,
            output,
            class_parameters,
            class_is_readonly,
            is_external,
        }
    }

    fn compile(&mut self, members: &[ClassLikeMember<'_>]) -> Result<(), CompileError> {
        for member in members {
            match member {
                ClassLikeMember::Constant(constant) => self.compile_constant(constant)?,
                ClassLikeMember::Property(property) => self.compile_property(property)?,
                ClassLikeMember::Method(method) => self.compile_method(method)?,
                ClassLikeMember::EnumCase(case) => self.compile_enum_case(case)?,
            }
        }
        Ok(())
    }

    fn static_scope(&self) -> Scope<'scope> {
        Scope {
            heap: self.scope.heap,
            runtime_path: self.scope.runtime_path,
            line_starts: self.scope.line_starts,
            resolver: self.scope.resolver,
            class: self.scope.class,
            binders: Vec::new(),
            forbidden_binders: self.scope.binders.clone(),
            generics: self.scope.generics,
            embedded_files: self.scope.embedded_files,
            trusted_returns: self.scope.trusted_returns,
        }
    }

    fn compile_constant(&mut self, constant: &ClassLikeConstant<'_>) -> Result<(), CompileError> {
        let heap = self.scope.heap;
        let static_scope = self.static_scope();
        let visibility = member_visibility(constant.modifiers, constant.span())?;
        let (declared_type, rendered_type) = match constant.r#type {
            Some(annotation) => {
                types::lowering::reject_return_only_annotation(annotation, "class constant")?;
                let type_scope = TypeScope {
                    heap,
                    resolver: self.scope.resolver,
                    class: Some(self.class_context),
                    aliases: &self.unit.type_aliases,
                    binders: &static_scope.binders,
                    forbidden_binders: &static_scope.forbidden_binders,
                    generics: self.scope.generics,
                };
                (
                    Some(lower_type(&type_scope, annotation)?),
                    Some(render_type(&type_scope, annotation)?),
                )
            }
            None => (None, None),
        };

        if !self.is_external {
            rules::check_constant_initializer(constant.value)?;
        }

        let initializer = compile_initializer(
            heap,
            &static_scope,
            constant.value,
            self.path,
            self.source_text,
            &mut DeclarationContext::for_unit(self.unit),
        )?;

        if !self.is_external
            && let (Some(descriptor), ConstantInitializer::Literal(literal)) =
                (&declared_type, &initializer)
            && literal_satisfies(literal, descriptor, &self.unit.type_aliases) == Some(false)
        {
            return Err(CompileError::new(
                CompileErrorKind::ClassConstantTypeMismatch,
                format!(
                    "the constant {} declares `{}`, and its value does not satisfy it",
                    constant.name.value,
                    rendered_type.unwrap_or_default()
                ),
                constant.span(),
            ));
        }

        let attributes = compile_attributes(
            heap,
            &static_scope,
            constant.attribute_lists,
            self.path,
            self.source_text,
            &mut DeclarationContext::for_unit(self.unit),
        )?;

        self.output.constants.push(CompiledClassConstant {
            name: heap.intern(constant.name.value.as_bytes()),
            span: constant.span(),
            visibility,
            initializer,
            declared_type,
            attributes,
        });

        Ok(())
    }

    fn compile_property(&mut self, property: &Property<'_>) -> Result<(), CompileError> {
        let heap = self.scope.heap;
        let static_scope = self.static_scope();
        let property_scope = if property.is_static() {
            &static_scope
        } else {
            self.scope
        };

        let visibility = member_visibility(property.modifiers, property.span())?;
        let is_readonly =
            !property.is_static() && (self.class_is_readonly || property.is_readonly());
        let declared_type = match property.r#type {
            Some(annotation) => {
                types::lowering::reject_return_only_annotation(annotation, "property")?;
                let type_scope = TypeScope {
                    heap,
                    resolver: self.scope.resolver,
                    class: Some(self.class_context),
                    aliases: &self.unit.type_aliases,
                    binders: &property_scope.binders,
                    forbidden_binders: &property_scope.forbidden_binders,
                    generics: self.scope.generics,
                };
                Some(lower_type(&type_scope, annotation)?)
            }
            None => None,
        };

        let default = if let (Some(default), false) = (&property.default, self.is_external) {
            rules::check_property_default(default.value)?;
            Some(compile_initializer(
                heap,
                property_scope,
                default.value,
                self.path,
                self.source_text,
                &mut DeclarationContext::for_unit(self.unit),
            )?)
        } else {
            None
        };

        if let Some(descriptor) = &declared_type {
            validate_variance_use(
                descriptor,
                i8::from(is_readonly),
                &self.class_parameters,
                self.scope.generics,
                property.span(),
            )?;
        }

        let attributes = compile_attributes(
            heap,
            property_scope,
            property.attribute_lists,
            self.path,
            self.source_text,
            &mut DeclarationContext::for_unit(self.unit),
        )?;

        self.output.properties.push(CompiledProperty {
            name: heap.intern(
                property
                    .variable
                    .name
                    .strip_prefix('$')
                    .unwrap_or(property.variable.name)
                    .as_bytes(),
            ),
            span: property.span(),
            visibility,
            is_static: property.is_static(),
            is_readonly,
            is_promoted: false,
            default,
            declared_type,
            attributes,
        });

        Ok(())
    }

    fn method_scope(&self, method: &Method<'_>) -> Scope<'scope> {
        let own_binders = binder_names(method.type_parameters.as_ref());
        let (binders, forbidden_binders) = if method.is_static() && !self.is_external {
            (own_binders, self.scope.binders.clone())
        } else {
            let mut binders = self.scope.binders.clone();
            binders.extend(own_binders);
            (binders, self.scope.forbidden_binders.clone())
        };

        Scope {
            heap: self.scope.heap,
            runtime_path: self.scope.runtime_path,
            line_starts: self.scope.line_starts,
            resolver: self.scope.resolver,
            class: self.scope.class,
            binders,
            forbidden_binders,
            generics: self.scope.generics,
            embedded_files: self.scope.embedded_files,
            trusted_returns: self.scope.trusted_returns,
        }
    }

    fn compile_method_metadata(
        &mut self,
        method: &Method<'_>,
        scope: &Scope<'_>,
        is_constructor: bool,
    ) -> Result<MethodMetadata, CompileError> {
        let heap = self.scope.heap;
        let parameters = compile_parameters(
            heap,
            scope,
            &method.parameter_list,
            self.path,
            self.source_text,
            &mut DeclarationContext::for_unit(self.unit),
        )?;

        if !is_constructor {
            validate_parameter_variance(
                &parameters,
                &self.class_parameters,
                self.scope.generics,
                method.span(),
            )?;
        }

        let attributes = compile_attributes(
            heap,
            scope,
            method.attribute_lists,
            self.path,
            self.source_text,
            &mut DeclarationContext::for_unit(self.unit),
        )?;

        let type_parameters = compile_type_parameters(
            heap,
            scope,
            &self.unit.type_aliases,
            method.type_parameters.as_ref(),
        )?;

        validate_parameter_variance(
            &parameters,
            &type_parameters,
            self.scope.generics,
            method.span(),
        )?;

        let type_scope = TypeScope {
            heap,
            resolver: self.scope.resolver,
            class: Some(self.class_context),
            aliases: &self.unit.type_aliases,
            binders: &scope.binders,
            forbidden_binders: &scope.forbidden_binders,
            generics: self.scope.generics,
        };

        let return_type = method
            .return_type
            .as_ref()
            .map(|annotation| {
                types::lowering::validate_return_annotation(annotation.r#type)?;
                lower_type(&type_scope, annotation.r#type)
            })
            .transpose()?;

        if let Some(descriptor) = &return_type {
            validate_variance_use(
                descriptor,
                1,
                &self.class_parameters,
                self.scope.generics,
                method.span(),
            )?;
            validate_variance_use(
                descriptor,
                1,
                &type_parameters,
                self.scope.generics,
                method.span(),
            )?;
        }

        let signature = render_signature(
            heap,
            scope,
            &self.unit.type_aliases,
            method.type_parameters.as_ref(),
            &method.parameter_list,
            method
                .return_type
                .as_ref()
                .map(|annotation| annotation.r#type),
        )?;

        let returns_void = matches!(
            method
                .return_type
                .as_ref()
                .map(|annotation| annotation.r#type.unparenthesized()),
            Some(Type::Void(_))
        );

        let returns_never = return_type.as_ref().is_some_and(|descriptor| {
            types::descriptor_is_never(descriptor, &self.unit.type_aliases)
        });

        Ok(MethodMetadata {
            parameters,
            attributes,
            type_parameters,
            return_type,
            signature,
            return_kind: ReturnKind::callable(returns_void, returns_never),
        })
    }

    fn compile_promoted_properties(
        &mut self,
        method: &Method<'_>,
        scope: &Scope<'_>,
    ) -> Result<(), CompileError> {
        let heap = self.scope.heap;
        for parameter in rules::promoted_properties(method) {
            let visibility = member_visibility(parameter.modifiers, parameter.span())?;
            let is_readonly = self.class_is_readonly
                || parameter
                    .modifiers
                    .iter()
                    .any(|modifier| matches!(modifier, Modifier::Readonly(_)));
            let declared_type = match parameter.r#type {
                Some(annotation) => {
                    let type_scope = TypeScope {
                        heap,
                        resolver: self.scope.resolver,
                        class: Some(self.class_context),
                        aliases: &self.unit.type_aliases,
                        binders: &scope.binders,
                        forbidden_binders: &scope.forbidden_binders,
                        generics: self.scope.generics,
                    };
                    Some(lower_type(&type_scope, annotation)?)
                }
                None => None,
            };

            if let Some(descriptor) = &declared_type {
                validate_variance_use(
                    descriptor,
                    i8::from(is_readonly),
                    &self.class_parameters,
                    self.scope.generics,
                    parameter.span(),
                )?;
            }

            let attributes = compile_attributes(
                heap,
                scope,
                parameter.attribute_lists,
                self.path,
                self.source_text,
                &mut DeclarationContext::for_unit(self.unit),
            )?;

            self.output.properties.push(CompiledProperty {
                name: heap.intern(
                    parameter
                        .variable
                        .name
                        .strip_prefix('$')
                        .unwrap_or(parameter.variable.name)
                        .as_bytes(),
                ),
                span: parameter.span(),
                visibility,
                is_static: false,
                is_readonly,
                is_promoted: true,
                default: None,
                declared_type,
                attributes,
            });
        }

        Ok(())
    }

    fn compile_method(&mut self, method: &Method<'_>) -> Result<(), CompileError> {
        let heap = self.scope.heap;
        let visibility = member_visibility(method.modifiers, method.span())?;
        let is_constructor = method.name.value == "__construct";
        let method_scope = self.method_scope(method);
        let metadata = self.compile_method_metadata(method, &method_scope, is_constructor)?;
        let chunk = if self.is_external {
            inert_declaration_chunk(method.name.span())
        } else {
            match &method.body {
                MethodBody::Abstract(_) => abstract_method_chunk(method.name.span()),
                MethodBody::Concrete(block) => compile_body(
                    &method_scope,
                    self.path,
                    self.source_text,
                    self.unit,
                    &method.parameter_list,
                    block,
                    BodyShape {
                        is_instance_method: !method.is_static(),
                        return_kind: metadata.return_kind,
                        promote_parameters: is_constructor && !method.is_static(),
                        trusted_returns: method_scope.trusted_returns,
                    },
                )?,
            }
        };

        self.compile_promoted_properties(method, &method_scope)?;
        self.output.methods.push(CompiledMethod {
            name: heap.intern(method.name.value.as_bytes()),
            visibility,
            is_static: method.is_static(),
            is_abstract: method.is_abstract(),
            is_final: method.is_final(),
            function: CompiledFunction {
                name: heap.intern(
                    format!("{}::{}", self.class_context.name, method.name.value).as_bytes(),
                ),
                span: method.span(),
                signature: heap.intern(metadata.signature.as_bytes()),
                type_parameters: metadata.type_parameters,
                parameters: metadata.parameters,
                return_type: metadata.return_type,
                attributes: metadata.attributes,
                captures_this: false,
                capture_names: Vec::new(),
                is_short_closure: false,
                capture_types: Vec::new(),
                chunk,
            },
        });

        Ok(())
    }

    fn compile_enum_case(&mut self, case: &EnumCase<'_>) -> Result<(), CompileError> {
        let heap = self.scope.heap;
        let attributes = compile_attributes(
            heap,
            self.scope,
            case.attribute_lists,
            self.path,
            self.source_text,
            &mut DeclarationContext::for_unit(self.unit),
        )?;

        let value = match (&case.value, self.is_external, self.output.enum_backing) {
            (Some(_), true, Some(EnumBacking::Int)) => {
                Some(ConstantInitializer::Literal(BytecodeLiteral::Int(0)))
            }
            (Some(_), true, Some(EnumBacking::String)) => Some(ConstantInitializer::Literal(
                BytecodeLiteral::String(heap.intern(b"")),
            )),
            (Some(value), false, _) => Some(compile_initializer(
                heap,
                self.scope,
                value.expression,
                self.path,
                self.source_text,
                &mut DeclarationContext::for_unit(self.unit),
            )?),
            (Some(_), true, None) | (None, _, _) => None,
        };

        if let (Some(backing), Some(ConstantInitializer::Literal(literal))) =
            (self.output.enum_backing, &value)
        {
            let satisfied = match backing {
                EnumBacking::Int => matches!(literal, BytecodeLiteral::Int(_)),
                EnumBacking::String => matches!(literal, BytecodeLiteral::String(_)),
            };
            if !satisfied {
                let expected = match backing {
                    EnumBacking::Int => "int",
                    EnumBacking::String => "string",
                };
                return Err(CompileError::new(
                    CompileErrorKind::EnumCaseValueMismatch,
                    format!(
                        "the case {} must carry {expected} backing, and its value does not satisfy it",
                        case.name.value
                    ),
                    case.span(),
                ));
            }
        }

        self.output.cases.push(CompiledEnumCase {
            name: heap.intern(case.name.value.as_bytes()),
            span: case.span(),
            value,
            attributes,
        });

        Ok(())
    }
}

/// One alias parameter's descriptor and the lexical environment in which it
/// was written. The explicit environment distinguishes an outer `T` supplied
/// to `Inner<T>` from `Inner`'s own parameter named `T`.
struct LiteralTypeBinding<'a> {
    name: &'a Atom,
    descriptor: &'a TypeDescriptor,
    parent: Option<usize>,
    descriptor_environment: Option<usize>,
}

/// Whether a folded literal satisfies a declared constant type, when that is
/// decidable at compile time. Same-unit aliases are structural shorthand, so
/// their concrete arguments and defaults get bound before the aliased
/// descriptor is inspected. `None` marks a nominal type the compiler cannot
/// decide.
fn literal_satisfies<'a>(
    literal: &BytecodeLiteral,
    descriptor: &'a TypeDescriptor,
    aliases: &'a [CompiledTypeAlias],
) -> Option<bool> {
    literal_satisfies_in(literal, descriptor, aliases, &mut Vec::new(), None, 0)
}

#[expect(
    clippy::too_many_lines,
    reason = "the exhaustive descriptor match is clearest in one place"
)]
#[expect(
    clippy::float_cmp,
    reason = "float literal types require exact equality"
)]
fn literal_satisfies_in<'a>(
    literal: &BytecodeLiteral,
    descriptor: &'a TypeDescriptor,
    aliases: &'a [CompiledTypeAlias],
    bindings: &mut Vec<LiteralTypeBinding<'a>>,
    environment: Option<usize>,
    depth: usize,
) -> Option<bool> {
    if depth > MAX_TYPE_DEPTH {
        return None;
    }

    Some(match descriptor {
        TypeDescriptor::Wildcard | TypeDescriptor::Mixed => true,
        TypeDescriptor::Null => matches!(literal, BytecodeLiteral::Null),
        TypeDescriptor::Bool => matches!(literal, BytecodeLiteral::Bool(_)),
        TypeDescriptor::Int => matches!(literal, BytecodeLiteral::Int(_)),
        TypeDescriptor::Float => matches!(literal, BytecodeLiteral::Float(_)),
        TypeDescriptor::String => matches!(literal, BytecodeLiteral::String(_)),
        TypeDescriptor::StringLength { min, max } => matches!(
            literal,
            BytecodeLiteral::String(value)
                if string_length_matches(value.as_bytes().len(), *min, *max)
        ),
        TypeDescriptor::Void
        | TypeDescriptor::Never
        | TypeDescriptor::Object
        | TypeDescriptor::StaticClass
        | TypeDescriptor::Array(_)
        | TypeDescriptor::Vector(_)
        | TypeDescriptor::Dictionary(_)
        | TypeDescriptor::Callable(_)
        | TypeDescriptor::Classname(_)
        | TypeDescriptor::Tuple(_)
        | TypeDescriptor::TupleRest { .. }
        | TypeDescriptor::TupleAny
        | TypeDescriptor::VectorShape { .. }
        | TypeDescriptor::DictionaryShape { .. } => false,
        TypeDescriptor::Member { .. } | TypeDescriptor::Intersection(_) => return None,
        TypeDescriptor::TrueLiteral => matches!(literal, BytecodeLiteral::Bool(true)),
        TypeDescriptor::FalseLiteral => matches!(literal, BytecodeLiteral::Bool(false)),
        TypeDescriptor::IntLiteral(expected) => {
            matches!(literal, BytecodeLiteral::Int(value) if value == expected)
        }
        TypeDescriptor::IntRange { min, max } => {
            matches!(
                literal,
                BytecodeLiteral::Int(value)
                    if min.is_none_or(|min| *value >= min)
                        && max.is_none_or(|max| *value <= max)
            )
        }
        TypeDescriptor::FloatLiteral(expected) => {
            matches!(literal, BytecodeLiteral::Float(value) if value == expected)
        }
        TypeDescriptor::StringLiteral(expected) => {
            matches!(literal, BytecodeLiteral::String(value) if value == expected)
        }
        TypeDescriptor::Named {
            name, arguments, ..
        } => {
            let alias = aliases
                .iter()
                .find(|alias| alias.name.as_bytes() == name.as_bytes())?;
            let arguments = arguments.as_deref().unwrap_or_default();
            if arguments.len() > alias.type_parameters.len() {
                return None;
            }

            let first_binding = bindings.len();
            let outer_environment = environment;
            let mut alias_environment = environment;
            for (position, parameter) in alias.type_parameters.iter().enumerate() {
                let (descriptor, descriptor_environment) =
                    if let Some(argument) = arguments.get(position) {
                        (argument, outer_environment)
                    } else {
                        let Some(default) = parameter.default.as_ref() else {
                            bindings.truncate(first_binding);
                            return None;
                        };
                        (default, alias_environment)
                    };
                let binding = bindings.len();
                bindings.push(LiteralTypeBinding {
                    name: &parameter.name,
                    descriptor,
                    parent: alias_environment,
                    descriptor_environment,
                });
                alias_environment = Some(binding);
            }

            let satisfied = literal_satisfies_in(
                literal,
                &alias.descriptor,
                aliases,
                bindings,
                alias_environment,
                depth + 1,
            );

            bindings.truncate(first_binding);
            return satisfied;
        }

        TypeDescriptor::Parameter(name) => {
            let mut current = environment;
            while let Some(binding) = current {
                let binding = &bindings[binding];
                if binding.name.as_bytes() == name.as_bytes() {
                    return literal_satisfies_in(
                        literal,
                        binding.descriptor,
                        aliases,
                        bindings,
                        binding.descriptor_environment,
                        depth + 1,
                    );
                }

                current = binding.parent;
            }

            return None;
        }
        TypeDescriptor::Negated(inner) => {
            return literal_satisfies_in(literal, inner, aliases, bindings, environment, depth + 1)
                .map(|satisfied| !satisfied);
        }
        TypeDescriptor::Union(members) => {
            let mut undecided = false;
            for member in members {
                match literal_satisfies_in(
                    literal,
                    member,
                    aliases,
                    bindings,
                    environment,
                    depth + 1,
                ) {
                    Some(true) => return Some(true),
                    Some(false) => {}
                    None => undecided = true,
                }
            }

            if undecided {
                return None;
            }

            false
        }
    })
}

/// A well-formed inert chunk for an abstract method.
fn abstract_method_chunk(span: Span) -> Chunk {
    let mut chunk = Chunk::new();
    chunk.emit(Instruction::ReturnNull, span);
    chunk
}
