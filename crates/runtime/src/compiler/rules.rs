//! Declaration legality: the modifier matrix, member-kind legality, abstract
//! shape, and enum shape.

use hashbrown::HashMap;

use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::cst::access::Access;
use whim_syn::cst::access::ClassReference;
use whim_syn::cst::array::DictEntry;
use whim_syn::cst::array::DictExpression;
use whim_syn::cst::array::TupleElement;
use whim_syn::cst::array::TupleExpression;
use whim_syn::cst::array::VecExpression;
use whim_syn::cst::atom::Literal;
use whim_syn::cst::atom::Modifier;
use whim_syn::cst::call::Argument;
use whim_syn::cst::call::Call;
use whim_syn::cst::call::Callee;
use whim_syn::cst::class::Class;
use whim_syn::cst::class::ClassLikeConstant;
use whim_syn::cst::class::ClassLikeMember;
use whim_syn::cst::class::Enum;
use whim_syn::cst::class::EnumCase;
use whim_syn::cst::class::Interface;
use whim_syn::cst::class::Method;
use whim_syn::cst::class::MethodBody;
use whim_syn::cst::class::Property;
use whim_syn::cst::construct::Construct;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::expression::Instantiation;
use whim_syn::cst::function::Parameter;
use whim_syn::cst::function::ParameterList;
use whim_syn::cst::operation::UnaryPrefixOperator;
use whim_syn::cst::r#type::Type;

use crate::compiler::emit::analysis::references_this_in_block;
use crate::compiler::emit::integer_gate;
use crate::compiler::error::CompileError;
use crate::compiler::error::CompileErrorKind;
use crate::compiler::limits::check_count;
use crate::compiler::limits::check_sequence;

/// Which declaration a modifier list is written on. Each context admits its
/// own set, which is what makes the matrix context-sensitive.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModifierContext {
    Class,
    Method,
    Property,
    ClassConstant,
    PromotedParameter,
}

impl ModifierContext {
    const fn describe(self) -> &'static str {
        match self {
            Self::Class => "a class declaration",
            Self::Method => "a method",
            Self::Property => "a property",
            Self::ClassConstant => "a class constant",
            Self::PromotedParameter => "a promoted constructor parameter",
        }
    }

    const fn admits(self, modifier: &Modifier<'_>) -> bool {
        match self {
            Self::Class => matches!(
                modifier,
                Modifier::Abstract(_) | Modifier::Final(_) | Modifier::Readonly(_)
            ),
            Self::Method => matches!(
                modifier,
                Modifier::Public(_)
                    | Modifier::Protected(_)
                    | Modifier::Private(_)
                    | Modifier::Static(_)
                    | Modifier::Final(_)
                    | Modifier::Abstract(_)
            ),
            Self::Property => matches!(
                modifier,
                Modifier::Public(_)
                    | Modifier::Protected(_)
                    | Modifier::Private(_)
                    | Modifier::Static(_)
                    | Modifier::Readonly(_)
            ),
            Self::ClassConstant => matches!(
                modifier,
                Modifier::Public(_)
                    | Modifier::Protected(_)
                    | Modifier::Private(_)
                    | Modifier::Final(_)
            ),
            Self::PromotedParameter => matches!(
                modifier,
                Modifier::Public(_)
                    | Modifier::Protected(_)
                    | Modifier::Private(_)
                    | Modifier::Readonly(_)
            ),
        }
    }
}

const fn spelling(modifier: &Modifier<'_>) -> &'static str {
    match modifier {
        Modifier::Public(_) => "public",
        Modifier::Protected(_) => "protected",
        Modifier::Private(_) => "private",
        Modifier::Static(_) => "static",
        Modifier::Final(_) => "final",
        Modifier::Abstract(_) => "abstract",
        Modifier::Readonly(_) => "readonly",
    }
}

const fn same_keyword(left: &Modifier<'_>, right: &Modifier<'_>) -> bool {
    matches!(
        (left, right),
        (Modifier::Public(_), Modifier::Public(_))
            | (Modifier::Protected(_), Modifier::Protected(_))
            | (Modifier::Private(_), Modifier::Private(_))
            | (Modifier::Static(_), Modifier::Static(_))
            | (Modifier::Final(_), Modifier::Final(_))
            | (Modifier::Abstract(_), Modifier::Abstract(_))
            | (Modifier::Readonly(_), Modifier::Readonly(_))
    )
}

/// Applies the modifier matrix to one modifier list: every modifier must be
/// admitted by the context, written once, and free of contradiction.
fn check_modifiers(
    modifiers: &[Modifier<'_>],
    context: ModifierContext,
) -> Result<(), CompileError> {
    for (index, modifier) in modifiers.iter().enumerate() {
        if !context.admits(modifier) {
            return Err(CompileError::new(
                CompileErrorKind::ModifierNotAllowed,
                format!(
                    "`{}` is not a modifier of {}",
                    spelling(modifier),
                    context.describe()
                ),
                modifier.span(),
            ));
        }
        if let Some(earlier) = modifiers[..index]
            .iter()
            .find(|earlier| same_keyword(earlier, modifier))
        {
            return Err(CompileError::new(
                CompileErrorKind::DuplicateModifier,
                format!("`{}` is written twice", spelling(modifier)),
                modifier.span(),
            )
            .with_note(earlier.span(), "it is already written here"));
        }
        if modifier.is_visibility()
            && let Some(earlier) = modifiers[..index]
                .iter()
                .find(|earlier| earlier.is_visibility())
        {
            return Err(CompileError::new(
                CompileErrorKind::ConflictingModifiers,
                format!(
                    "`{}` conflicts with `{}`; a declaration has one visibility",
                    spelling(modifier),
                    spelling(earlier)
                ),
                modifier.span(),
            )
            .with_note(earlier.span(), "the visibility is already given here"));
        }
    }
    if context != ModifierContext::Class {
        conflicting_pair(modifiers, "abstract", "final")?;
    }
    conflicting_pair(modifiers, "static", "readonly")?;
    conflicting_pair(modifiers, "abstract", "private")?;

    Ok(())
}

fn conflicting_pair(
    modifiers: &[Modifier<'_>],
    left: &str,
    right: &str,
) -> Result<(), CompileError> {
    let found_left = modifiers.iter().find(|modifier| spelling(modifier) == left);
    let found_right = modifiers
        .iter()
        .find(|modifier| spelling(modifier) == right);
    if let (Some(first), Some(second)) = (found_left, found_right) {
        let reason = match (left, right) {
            ("abstract", "final") => "an abstract declaration exists to be overridden",
            ("static", "readonly") => {
                "`readonly` describes an instance property written once during construction, \
                 and a static property has no construction"
            }
            ("abstract", "private") => "an abstract member cannot be implemented outside its class",
            _ => "the two modifiers contradict each other",
        };
        let span = if first.span().start.offset > second.span().start.offset {
            first.span()
        } else {
            second.span()
        };
        return Err(CompileError::new(
            CompileErrorKind::ConflictingModifiers,
            format!("`{left}` and `{right}` cannot be combined: {reason}"),
            span,
        ));
    }
    Ok(())
}

pub(in crate::compiler) fn check_class(class: &Class<'_>) -> Result<(), CompileError> {
    check_modifiers(class.modifiers, ModifierContext::Class)?;
    check_members(class.members, DeclarationKind::Class)?;

    if class.is_abstract() && class.is_final() {
        for member in class.members {
            let invalid = match member {
                ClassLikeMember::Property(property) => !property.is_static(),
                ClassLikeMember::Method(method) => method.is_abstract() || !method.is_static(),
                ClassLikeMember::Constant(_) | ClassLikeMember::EnumCase(_) => false,
            };
            if invalid {
                return Err(CompileError::new(
                    CompileErrorKind::InvalidStaticOnlyClassMember,
                    "a final abstract class may declare only concrete static methods, static \
                     properties, and constants",
                    member.span(),
                ));
            }
        }
    }

    if let Some(readonly) = class
        .modifiers
        .iter()
        .find(|modifier| matches!(modifier, Modifier::Readonly(_)))
    {
        for member in class.members {
            let ClassLikeMember::Property(property) = member else {
                continue;
            };
            let Some(static_modifier) = property
                .modifiers
                .iter()
                .find(|modifier| matches!(modifier, Modifier::Static(_)))
            else {
                continue;
            };

            return Err(CompileError::new(
                CompileErrorKind::ConflictingModifiers,
                "a readonly class cannot declare a static property; readonly classes contain \
                 only readonly instance state",
                static_modifier.span(),
            )
            .with_note(readonly.span(), "the class is declared readonly here"));
        }
    }

    Ok(())
}

pub(in crate::compiler) fn check_interface(interface: &Interface<'_>) -> Result<(), CompileError> {
    check_members(interface.members, DeclarationKind::Interface)
}

pub(in crate::compiler) fn check_enum(declaration: &Enum<'_>) -> Result<(), CompileError> {
    check_members(declaration.members, DeclarationKind::Enum)?;
    check_enum_built_in_methods(declaration)?;
    check_enum_shape(declaration)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeclarationKind {
    Class,
    Interface,
    Enum,
}

fn check_enum_built_in_methods(declaration: &Enum<'_>) -> Result<(), CompileError> {
    for member in declaration.members {
        let ClassLikeMember::Method(method) = member else {
            continue;
        };
        let built_in = method.name.value == "cases"
            || (declaration.is_backed() && matches!(method.name.value, "from" | "tryFrom"));
        if !built_in {
            continue;
        }

        return Err(CompileError::new(
            CompileErrorKind::EnumBuiltInMethodRedeclaration,
            format!(
                "an enum cannot redeclare the built-in method `{}`",
                method.name.value
            ),
            method.name.span(),
        ));
    }

    Ok(())
}

impl DeclarationKind {
    const fn describe(self) -> &'static str {
        match self {
            Self::Class => "a class",
            Self::Interface => "an interface",
            Self::Enum => "an enum",
        }
    }
}

pub(in crate::compiler) fn promoted_properties<'arena>(
    method: &Method<'arena>,
) -> impl Iterator<Item = &'arena Parameter<'arena>> {
    let parameters =
        (method.name.value == "__construct").then(|| method.parameter_list.parameters.as_slice());

    parameters
        .into_iter()
        .flatten()
        .filter(|parameter| parameter.is_promoted_property())
}

fn check_promoted_properties<'arena>(
    method: &Method<'arena>,
    kind: DeclarationKind,
    properties: &mut Vec<(&'arena str, Span)>,
    declares: &str,
) -> Result<(), CompileError> {
    for parameter in promoted_properties(method) {
        if kind != DeclarationKind::Class {
            return Err(CompileError::new(
                CompileErrorKind::MemberNotAllowed,
                format!(
                    "{} cannot declare a property, and promoting a parameter declares one",
                    kind.describe()
                ),
                parameter.span(),
            ));
        }
        if method.is_static() {
            return Err(CompileError::new(
                CompileErrorKind::ConflictingModifiers,
                "`static` and a promoted parameter cannot be combined: promotion declares an \
                 instance property, and a static method has no instance to write it on",
                parameter.span(),
            ));
        }
        note_member(
            properties,
            parameter.variable.name,
            parameter.variable.span(),
            "property",
        )?;
        check_count(
            CompileErrorKind::TooManyMembers,
            declares,
            "properties of its own",
            properties.len(),
            parameter.variable.span(),
        )?;
    }

    Ok(())
}

fn check_members(
    members: &[ClassLikeMember<'_>],
    kind: DeclarationKind,
) -> Result<(), CompileError> {
    let mut state = MemberState::new(kind);
    for member in members {
        state.check(member)?;
    }

    Ok(())
}

struct MemberState<'arena> {
    kind: DeclarationKind,
    symbols: Vec<(&'arena str, Span, &'static str)>,
    properties: Vec<(&'arena str, Span)>,
    method_count: usize,
    constant_count: usize,
    declares: String,
}

impl<'arena> MemberState<'arena> {
    fn new(kind: DeclarationKind) -> Self {
        Self {
            kind,
            symbols: Vec::new(),
            properties: Vec::new(),
            method_count: 0,
            constant_count: 0,
            declares: format!("{} may declare", kind.describe()),
        }
    }

    fn check(&mut self, member: &ClassLikeMember<'arena>) -> Result<(), CompileError> {
        match member {
            ClassLikeMember::Property(property) => self.check_property(property),
            ClassLikeMember::Method(method) => self.check_method(method),
            ClassLikeMember::Constant(constant) => self.check_constant(constant),
            ClassLikeMember::EnumCase(case) => self.check_enum_case(case),
        }
    }

    fn check_property(&mut self, property: &Property<'arena>) -> Result<(), CompileError> {
        if self.kind == DeclarationKind::Enum {
            return Err(CompileError::new(
                CompileErrorKind::MemberNotAllowed,
                format!("{} cannot declare a property", self.kind.describe()),
                property.span(),
            ));
        }
        check_modifiers(property.modifiers, ModifierContext::Property)?;
        if self.kind == DeclarationKind::Interface {
            check_interface_property(property)?;
        }
        note_member(
            &mut self.properties,
            property.variable.name,
            property.variable.span(),
            "property",
        )?;
        check_count(
            CompileErrorKind::TooManyMembers,
            &self.declares,
            "properties of its own",
            self.properties.len(),
            property.variable.span(),
        )?;

        Ok(())
    }

    fn check_method(&mut self, method: &Method<'arena>) -> Result<(), CompileError> {
        check_modifiers(method.modifiers, ModifierContext::Method)?;
        if self.kind == DeclarationKind::Enum && method.name.value == "__construct" {
            return Err(CompileError::new(
                CompileErrorKind::MemberNotAllowed,
                "an enum cannot declare a constructor; its cases are its only instances and they are constructed from their values",
                method.name.span(),
            ));
        }
        check_lifecycle_method(method)?;
        check_abstract_shape(method, self.kind)?;
        check_parameter_count(&method.parameter_list)?;
        check_parameter_modifiers(method.name.value, &method.parameter_list)?;
        check_promoted_properties(method, self.kind, &mut self.properties, &self.declares)?;
        note_symbol(
            &mut self.symbols,
            method.name.value,
            method.name.span(),
            "method",
        )?;
        self.method_count += 1;
        check_count(
            CompileErrorKind::TooManyMembers,
            &self.declares,
            "methods of its own",
            self.method_count,
            method.name.span(),
        )?;

        Ok(())
    }

    fn check_constant(&mut self, constant: &ClassLikeConstant<'arena>) -> Result<(), CompileError> {
        check_modifiers(constant.modifiers, ModifierContext::ClassConstant)?;
        note_symbol(
            &mut self.symbols,
            constant.name.value,
            constant.name.span(),
            "constant",
        )?;
        self.constant_count += 1;
        check_count(
            CompileErrorKind::TooManyMembers,
            &self.declares,
            "constants of its own",
            self.constant_count,
            constant.name.span(),
        )?;

        Ok(())
    }

    fn check_enum_case(&mut self, case: &EnumCase<'arena>) -> Result<(), CompileError> {
        if self.kind != DeclarationKind::Enum {
            return Err(CompileError::new(
                CompileErrorKind::MemberNotAllowed,
                format!(
                    "{} cannot declare a case; only an enum has cases",
                    self.kind.describe()
                ),
                case.span(),
            ));
        }
        note_symbol(&mut self.symbols, case.name.value, case.name.span(), "case")
    }
}

fn check_interface_property(property: &Property<'_>) -> Result<(), CompileError> {
    if !property
        .modifiers
        .iter()
        .any(|modifier| matches!(modifier, Modifier::Public(_)))
    {
        return Err(CompileError::new(
            CompileErrorKind::InvalidInterfaceProperty,
            "an interface property must be public",
            property.span(),
        ));
    }
    if property.is_static() {
        return Err(CompileError::new(
            CompileErrorKind::InvalidInterfaceProperty,
            "an interface property must be an instance property",
            property.span(),
        ));
    }
    if property.r#type.is_none() {
        return Err(CompileError::new(
            CompileErrorKind::InvalidInterfaceProperty,
            "an interface property must declare its type",
            property.span(),
        ));
    }
    if let Some(default) = &property.default {
        return Err(CompileError::new(
            CompileErrorKind::InvalidInterfaceProperty,
            "an interface property is a requirement and cannot declare a default",
            default.span(),
        ));
    }

    Ok(())
}

fn note_symbol<'source>(
    seen: &mut Vec<(&'source str, Span, &'static str)>,
    name: &'source str,
    span: Span,
    kind: &'static str,
) -> Result<(), CompileError> {
    if let Some((_, first, first_kind)) = seen.iter().find(|(seen_name, _, _)| *seen_name == name) {
        let message = if *first_kind == kind {
            format!("the {kind} `{name}` is declared twice")
        } else {
            format!("the {kind} `{name}` conflicts with the {first_kind} of the same name")
        };
        return Err(
            CompileError::new(CompileErrorKind::DuplicateMember, message, span).with_note(
                *first,
                format!("the {first_kind} `{name}` is declared here"),
            ),
        );
    }
    seen.push((name, span, kind));

    Ok(())
}

/// Enforces the fixed source-level shapes of the two object lifecycle methods.
fn check_lifecycle_method(method: &Method<'_>) -> Result<(), CompileError> {
    match method.name.value {
        "__construct" => {
            if let Some(return_type) = &method.return_type {
                return Err(CompileError::new(
                    CompileErrorKind::InvalidLifecycleMethod,
                    "a constructor cannot declare a return type",
                    return_type.span(),
                ));
            }
        }
        "__destruct" => {
            if method.is_static() {
                return Err(CompileError::new(
                    CompileErrorKind::InvalidLifecycleMethod,
                    "a destructor cannot be static",
                    method.name.span(),
                ));
            }
            if !method
                .modifiers
                .iter()
                .any(|modifier| matches!(modifier, Modifier::Public(_)))
            {
                return Err(CompileError::new(
                    CompileErrorKind::InvalidLifecycleMethod,
                    "a destructor must be public",
                    method.name.span(),
                ));
            }
            if !method.parameter_list.parameters.is_empty() {
                return Err(CompileError::new(
                    CompileErrorKind::InvalidLifecycleMethod,
                    "a destructor cannot declare parameters",
                    method.parameter_list.span(),
                ));
            }
            if let Some(type_parameters) = method.type_parameters
                && !type_parameters.parameters.is_empty()
            {
                return Err(CompileError::new(
                    CompileErrorKind::InvalidLifecycleMethod,
                    "a destructor cannot declare type parameters",
                    type_parameters.span(),
                ));
            }
            if let Some(return_type) = &method.return_type
                && !matches!(return_type.r#type.unparenthesized(), Type::Void(_))
            {
                return Err(CompileError::new(
                    CompileErrorKind::InvalidLifecycleMethod,
                    "a destructor may only declare the return type void",
                    return_type.span(),
                ));
            }
        }
        _ => {}
    }

    Ok(())
}

fn note_member<'source>(
    seen: &mut Vec<(&'source str, Span)>,
    name: &'source str,
    span: Span,
    kind: &str,
) -> Result<(), CompileError> {
    if let Some((_, first)) = seen.iter().find(|(seen_name, _)| *seen_name == name) {
        return Err(CompileError::new(
            CompileErrorKind::DuplicateMember,
            format!("the {kind} `{name}` is declared twice"),
            span,
        )
        .with_note(*first, format!("`{name}` is already declared here")));
    }
    seen.push((name, span));

    Ok(())
}

/// Requires an abstract method to end in a semicolon and a concrete one to
/// carry a body. An interface method may be written either way: without a body
/// it is a requirement, with one it is a default.
fn check_abstract_shape(method: &Method<'_>, kind: DeclarationKind) -> Result<(), CompileError> {
    let declared_abstract = method
        .modifiers
        .iter()
        .any(|modifier| matches!(modifier, Modifier::Abstract(_)));
    if declared_abstract && kind == DeclarationKind::Enum {
        return Err(CompileError::new(
            CompileErrorKind::ModifierNotAllowed,
            format!(
                "the method `{}` cannot be abstract; an enum's cases are instances, so it \
                 declares no method it does not implement",
                method.name.value
            ),
            method.name.span(),
        ));
    }
    match (&method.body, declared_abstract) {
        (MethodBody::Concrete(block), true) => Err(CompileError::new(
            CompileErrorKind::AbstractBodyMismatch,
            format!(
                "the abstract method `{}` cannot have a body; an abstract method ends in a \
                 semicolon",
                method.name.value
            ),
            block.span(),
        )),
        (MethodBody::Abstract(semicolon), false) if kind != DeclarationKind::Interface => {
            Err(CompileError::new(
                CompileErrorKind::AbstractBodyMismatch,
                format!(
                    "the method `{}` has no body; declare it `abstract`, or give it one",
                    method.name.value
                ),
                *semicolon,
            ))
        }
        _ => Ok(()),
    }
}

/// Refuses a parameter list longer than one callable may declare.
pub(in crate::compiler) fn check_parameter_count(
    parameter_list: &ParameterList<'_>,
) -> Result<(), CompileError> {
    check_sequence(
        CompileErrorKind::TooManyParameters,
        "a callable may declare",
        "parameters",
        &parameter_list.parameters,
    )
}

fn check_parameter_modifiers(
    method_name: &str,
    parameter_list: &ParameterList<'_>,
) -> Result<(), CompileError> {
    if method_name == "__construct" {
        for parameter in &parameter_list.parameters {
            check_modifiers(parameter.modifiers, ModifierContext::PromotedParameter)?;
        }
        return Ok(());
    }
    for parameter in &parameter_list.parameters {
        if let Some(modifier) = parameter.modifiers.first() {
            let message = if modifier.is_static() {
                "`static` may not be used as a parameter type; use `self` to accept an instance \
                 of the declaring class"
                    .to_owned()
            } else {
                format!(
                    "`{}` promotes a parameter to a property, which only a constructor can do",
                    spelling(modifier)
                )
            };
            return Err(CompileError::new(
                CompileErrorKind::ParameterModifierOutsideConstructor,
                message,
                modifier.span(),
            ));
        }
    }

    Ok(())
}

pub(in crate::compiler) fn check_free_function_parameters(
    parameter_list: &ParameterList<'_>,
) -> Result<(), CompileError> {
    for parameter in &parameter_list.parameters {
        if let Some(modifier) = parameter.modifiers.first() {
            return Err(CompileError::new(
                CompileErrorKind::ParameterModifierOutsideConstructor,
                format!(
                    "`{}` promotes a parameter to a property, and a function declares no class",
                    spelling(modifier)
                ),
                modifier.span(),
            ));
        }
    }

    Ok(())
}

/// Requires every case of a backed enum to carry a value of the declared
/// kind, no case of an unbacked enum to carry one, and every backing value to
/// be distinct.
fn check_enum_shape(declaration: &Enum<'_>) -> Result<(), CompileError> {
    let backed = declaration.backing_type.is_some();
    let mut values = HashMap::new();
    for member in declaration.members {
        let ClassLikeMember::EnumCase(case) = member else {
            continue;
        };
        match (&case.value, backed) {
            (None, true) => {
                return Err(CompileError::new(
                    CompileErrorKind::EnumCaseValueMissing,
                    format!(
                        "the case `{}` has no value; every case of a backed enum is valued",
                        case.name.value
                    ),
                    case.span(),
                ));
            }
            (Some(value), false) => {
                return Err(CompileError::new(
                    CompileErrorKind::EnumCaseValueNotAllowed,
                    format!(
                        "the case `{}` has a value, and this enum declares no backing type",
                        case.name.value
                    ),
                    value.expression.span(),
                ));
            }
            (Some(value), true) => {
                if let Some(key) = literal_key(value.expression)?
                    && let Some(first) = values.insert(key, value.expression.span())
                {
                    return Err(CompileError::new(
                        CompileErrorKind::DuplicateEnumCaseValue,
                        format!(
                            "the backing value of `{}` is already used by another case",
                            case.name.value
                        ),
                        value.expression.span(),
                    )
                    .with_note(first, "the same value is used here"));
                }
            }
            (None, false) => {}
        }
    }

    Ok(())
}

#[derive(PartialEq, Eq, Hash)]
enum EnumBackingKey<'source> {
    Int(i64),
    String(&'source [u8]),
}

/// A non-allocating key for a backing value the literal folder decides at
/// compile time. Other constant expressions are checked after evaluation by
/// the linker.
fn literal_key<'source>(
    expression: &'source Expression<'_>,
) -> Result<Option<EnumBackingKey<'source>>, CompileError> {
    match expression.unparenthesized() {
        Expression::Literal(Literal::Integer(integer)) => Ok(Some(EnumBackingKey::Int(
            integer_gate(integer.value, false, integer.span)?,
        ))),
        Expression::Literal(Literal::String(string)) => {
            Ok(Some(EnumBackingKey::String(string.value)))
        }
        Expression::UnaryPrefix(unary)
            if matches!(unary.operator, UnaryPrefixOperator::Negation(_)) =>
        {
            match unary.operand.unparenthesized() {
                Expression::Literal(Literal::Integer(integer)) => Ok(Some(EnumBackingKey::Int(
                    integer_gate(integer.value, true, unary.span())?,
                ))),
                _ => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

#[derive(Clone, Copy)]
enum ConstantExpressionPosition {
    AttributeArgument,
    ParameterDefault,
    ConstantInitializer,
    PropertyDefault,
}

pub(in crate::compiler) fn check_constant_expression(
    expression: &Expression<'_>,
) -> Result<(), CompileError> {
    check_constant_expression_at(expression, ConstantExpressionPosition::AttributeArgument)
}

pub(in crate::compiler) fn check_parameter_default(
    expression: &Expression<'_>,
) -> Result<(), CompileError> {
    check_constant_expression_at(expression, ConstantExpressionPosition::ParameterDefault)
}

pub(in crate::compiler) fn check_constant_initializer(
    expression: &Expression<'_>,
) -> Result<(), CompileError> {
    check_constant_expression_at(expression, ConstantExpressionPosition::ConstantInitializer)
}

pub(in crate::compiler) fn check_property_default(
    expression: &Expression<'_>,
) -> Result<(), CompileError> {
    check_constant_expression_at(expression, ConstantExpressionPosition::PropertyDefault)
}

fn check_constant_expression_at(
    expression: &Expression<'_>,
    position: ConstantExpressionPosition,
) -> Result<(), CompileError> {
    use whim_syn::cst::expression::Expression;

    match expression {
        Expression::Literal(_)
        | Expression::Access(Access::Constant(_) | Access::ClassConstant(_))
        | Expression::Construct(Construct::Embed(_)) => Ok(()),
        Expression::Parenthesized(parenthesized) => {
            check_constant_expression_at(parenthesized.expression, position)
        }
        Expression::Binary(binary) => {
            check_constant_expression_at(binary.lhs, position)?;
            check_constant_expression_at(binary.rhs, position)
        }
        Expression::UnaryPrefix(unary) => check_constant_expression_at(unary.operand, position),
        Expression::Vec(vector) => check_constant_vec(vector, position),
        Expression::Dict(dictionary) => check_constant_dict(dictionary, position),
        Expression::Tuple(tuple) => check_constant_tuple(tuple, position),
        Expression::Instantiation(instantiation)
            if !matches!(instantiation.class, ClassReference::Expression(_)) =>
        {
            check_constant_instantiation(instantiation, position)
        }
        Expression::Call(call) => check_constant_call(call, position),
        Expression::Closure(closure)
            if closure.use_clause.is_none() && !references_this_in_block(&closure.body) =>
        {
            Ok(())
        }
        other => Err(non_constant_expression_error(other, position)),
    }
}

fn check_constant_vec(
    vector: &VecExpression<'_>,
    position: ConstantExpressionPosition,
) -> Result<(), CompileError> {
    for element in vector.elements {
        check_constant_expression_at(element.value, position)?;
    }

    Ok(())
}

fn check_constant_dict(
    dictionary: &DictExpression<'_>,
    position: ConstantExpressionPosition,
) -> Result<(), CompileError> {
    for entry in dictionary.entries {
        match entry {
            DictEntry::Pair(pair) => {
                check_constant_expression_at(pair.key, position)?;
                check_constant_expression_at(pair.value, position)?;
            }
            DictEntry::Spread(spread) => {
                check_constant_expression_at(spread.value, position)?;
            }
        }
    }

    Ok(())
}

fn check_constant_tuple(
    tuple: &TupleExpression<'_>,
    position: ConstantExpressionPosition,
) -> Result<(), CompileError> {
    for element in tuple.elements {
        match element {
            TupleElement::Value(value) => check_constant_expression_at(value, position)?,
            TupleElement::Rest(rest) => {
                if let Some(value) = rest.value {
                    check_constant_expression_at(value, position)?;
                }
            }
        }
    }

    Ok(())
}

fn check_constant_instantiation(
    instantiation: &Instantiation<'_>,
    position: ConstantExpressionPosition,
) -> Result<(), CompileError> {
    if let Some(argument_list) = &instantiation.argument_list {
        for argument in &argument_list.arguments {
            let value = match argument {
                Argument::Positional(argument) => argument.value,
                Argument::Named(argument) => argument.value,
            };
            check_constant_expression_at(value, position)?;
        }
    }

    Ok(())
}

fn check_constant_call(
    call: &Call<'_>,
    position: ConstantExpressionPosition,
) -> Result<(), CompileError> {
    match call {
        Call::Function(call) => {
            if let Callee::Expression(expression) = call.function {
                check_constant_expression_at(expression, position)?;
            }
        }
        Call::Method(call) => check_constant_expression_at(call.object, position)?,
        Call::NullSafeMethod(call) => check_constant_expression_at(call.object, position)?,
        Call::StaticMethod(call) => {
            if let ClassReference::Expression(expression) = call.class {
                check_constant_expression_at(expression, position)?;
            }
        }
    }

    for argument in &call.get_argument_list().arguments {
        check_constant_expression_at(argument.value(), position)?;
    }

    Ok(())
}

fn non_constant_expression_error(
    expression: &Expression<'_>,
    position: ConstantExpressionPosition,
) -> CompileError {
    let what = match expression {
        Expression::Instantiation(_) => "a dynamically named instantiation",
        Expression::Variable(_) => "a variable",
        Expression::InterpolatedString(_) => "an interpolated string",
        Expression::Assignment(_) => "an assignment",
        Expression::Closure(closure) if closure.use_clause.is_some() => {
            "a closure with a `use` clause"
        }
        Expression::Closure(_) => "a closure that captures `$this`",
        Expression::ShortClosure(_) => "a short closure",
        Expression::Match(_) => "a match",
        Expression::Break(_) => "a break",
        Expression::Continue(_) => "a continue",
        Expression::Return(_) => "a return",
        Expression::Throw(_) => "a throw",
        Expression::Construct(_) => "a construct",
        Expression::VecFill(_) => "a filled vec",
        Expression::PartialApplication(_) => "a partial application",
        Expression::ArrayAccess(_) => "an index",
        Expression::Access(_) => "a property access",
        Expression::UnaryPostfix(_) => "an increment or decrement",
        Expression::TypeOperation(_) => "a type operation",
        _ => "this",
    };
    let (kind, message) = match position {
        ConstantExpressionPosition::AttributeArgument => (
            CompileErrorKind::NonConstantAttributeArgument,
            format!(
                "an attribute argument is a constant expression, and {what} is not one; an \
                 attribute describes a declaration, so its arguments cannot depend on when they \
                 are read"
            ),
        ),
        ConstantExpressionPosition::ParameterDefault => (
            CompileErrorKind::NonConstantParameterDefault,
            format!(
                "a parameter default is a constant expression, and {what} is not one; parameter \
                 defaults cannot depend on caller state"
            ),
        ),
        ConstantExpressionPosition::ConstantInitializer => (
            CompileErrorKind::NonConstantInitializer,
            format!(
                "a constant initializer is a constant expression, and {what} is not one; \
                 constants cannot read variables"
            ),
        ),
        ConstantExpressionPosition::PropertyDefault => (
            CompileErrorKind::NonConstantPropertyDefault,
            format!(
                "a property default is a constant expression, and {what} is not one; property \
                 defaults cannot read variables"
            ),
        ),
    };

    CompileError::new(kind, message, expression.span())
}
