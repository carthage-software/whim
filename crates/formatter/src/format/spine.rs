//! Iterative flattening of left-associative expression spines, so long chains
//! format at constant stack depth.

use std::vec::Vec;

use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::arena::Arena;
use whim_syn::cst::access::Access;
use whim_syn::cst::access::ClassConstantAccess;
use whim_syn::cst::access::ClassReference;
use whim_syn::cst::access::NullSafePropertyAccess;
use whim_syn::cst::access::PropertyAccess;
use whim_syn::cst::access::StaticPropertyAccess;
use whim_syn::cst::array::ArrayAccess;
use whim_syn::cst::call::Call;
use whim_syn::cst::call::Callee;
use whim_syn::cst::call::FunctionCall;
use whim_syn::cst::call::FunctionPartialApplication;
use whim_syn::cst::call::MethodCall;
use whim_syn::cst::call::MethodPartialApplication;
use whim_syn::cst::call::NullSafeMethodCall;
use whim_syn::cst::call::PartialApplication;
use whim_syn::cst::call::StaticMethodCall;
use whim_syn::cst::call::StaticMethodPartialApplication;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::operation::AssignmentTarget;
use whim_syn::cst::operation::Binary;
use whim_syn::cst::operation::BinaryOperator;
use whim_syn::cst::operation::TypeOperation;
use whim_syn::cst::operation::TypeOperator;
use whim_syn::cst::operation::UnaryPostfix;

use crate::document::BreakMode;
use crate::document::Document;
use crate::document::Group;
use crate::format::Format;
use crate::format::FormatterState;
use crate::format::expression::type_operator_text;

enum SpineLink<'source, 'arena> {
    Binary(&'source Binary<'arena>),
    Index(&'source ArrayAccess<'arena>),
    Property(&'source PropertyAccess<'arena>),
    NullSafeProperty(&'source NullSafePropertyAccess<'arena>),
    Method(&'source MethodCall<'arena>),
    NullSafeMethod(&'source NullSafeMethodCall<'arena>),
    Postfix(&'source UnaryPostfix<'arena>),
    TypeCheck(&'source TypeOperation<'arena>),
    Call(&'source FunctionCall<'arena>),
    PartialCall(&'source FunctionPartialApplication<'arena>),
    StaticProperty(&'source StaticPropertyAccess<'arena>),
    ClassConstant(&'source ClassConstantAccess<'arena>),
    StaticMethod(&'source StaticMethodCall<'arena>),
    PartialMethod(&'source MethodPartialApplication<'arena>),
    PartialStaticMethod(&'source StaticMethodPartialApplication<'arena>),
}

enum SpineFoot<'source, 'arena> {
    Expression(&'source Expression<'arena>),
    Callee(&'source Callee<'arena>),
    Class(&'source ClassReference<'arena>),
}

/// Formats a left-associative spine, `$a + $b + $c`, `$a->b[0]->c()`, and any
/// mixture of the two into one flat document, or returns `None` when the
/// expression does not head a spine.
pub(in crate::format) fn format_spine<'arena, A>(
    expression: &Expression<'arena>,
    f: &mut FormatterState<'arena, A>,
) -> Option<Document<'arena, A>>
where
    A: Arena,
{
    let mut links = Vec::new();
    let foot = collect_expression_spine(expression, &mut links);

    if links.is_empty() {
        return None;
    }

    Some(render_spine(expression.leftmost_span(), foot, links, f))
}

/// Formats the access-like assignment targets that share expression-spine
/// syntax without giving those node kinds a second renderer.
pub(in crate::format) fn format_assignment_target_spine<'arena, A>(
    target: &AssignmentTarget<'arena>,
    f: &mut FormatterState<'arena, A>,
) -> Option<Document<'arena, A>>
where
    A: Arena,
{
    let mut links = Vec::new();
    let foot = match target {
        AssignmentTarget::Property(access) => {
            links.push(SpineLink::Property(access));
            collect_expression_spine(access.object, &mut links)
        }
        AssignmentTarget::StaticProperty(access) => {
            links.push(SpineLink::StaticProperty(access));
            match &access.class {
                ClassReference::Expression(class) => collect_expression_spine(class, &mut links),
                class => SpineFoot::Class(class),
            }
        }
        AssignmentTarget::ArrayIndex(access) => {
            links.push(SpineLink::Index(access));
            collect_expression_spine(access.array, &mut links)
        }
        _ => return None,
    };

    Some(render_spine(target.leftmost_span(), foot, links, f))
}

fn collect_expression_spine<'source, 'arena>(
    expression: &'source Expression<'arena>,
    links: &mut Vec<SpineLink<'source, 'arena>>,
) -> SpineFoot<'source, 'arena> {
    let mut current = expression;
    loop {
        match current {
            Expression::Binary(binary) => {
                links.push(SpineLink::Binary(binary));
                current = binary.lhs;
            }
            Expression::ArrayAccess(access) => {
                links.push(SpineLink::Index(access));
                current = access.array;
            }
            Expression::Access(Access::Property(access)) => {
                links.push(SpineLink::Property(access));
                current = access.object;
            }
            Expression::Access(Access::NullSafeProperty(access)) => {
                links.push(SpineLink::NullSafeProperty(access));
                current = access.object;
            }
            Expression::Call(Call::Method(call)) => {
                links.push(SpineLink::Method(call));
                current = call.object;
            }
            Expression::Call(Call::NullSafeMethod(call)) => {
                links.push(SpineLink::NullSafeMethod(call));
                current = call.object;
            }
            Expression::UnaryPostfix(postfix) => {
                links.push(SpineLink::Postfix(postfix));
                current = postfix.operand;
            }
            Expression::TypeOperation(operation) => {
                links.push(SpineLink::TypeCheck(operation));
                current = operation.operand;
            }
            Expression::Call(Call::Function(call)) => {
                links.push(SpineLink::Call(call));
                match &call.function {
                    Callee::Expression(callee) => current = callee,
                    callee @ Callee::Identifier(_) => break SpineFoot::Callee(callee),
                }
            }
            Expression::PartialApplication(PartialApplication::Function(application)) => {
                links.push(SpineLink::PartialCall(application));
                match &application.function {
                    Callee::Expression(callee) => current = callee,
                    callee @ Callee::Identifier(_) => break SpineFoot::Callee(callee),
                }
            }
            Expression::Access(Access::StaticProperty(access)) => {
                links.push(SpineLink::StaticProperty(access));
                match &access.class {
                    ClassReference::Expression(class) => current = class,
                    class => break SpineFoot::Class(class),
                }
            }
            Expression::Access(Access::ClassConstant(access)) => {
                links.push(SpineLink::ClassConstant(access));
                match &access.class {
                    ClassReference::Expression(class) => current = class,
                    class => break SpineFoot::Class(class),
                }
            }
            Expression::Call(Call::StaticMethod(call)) => {
                links.push(SpineLink::StaticMethod(call));
                match &call.class {
                    ClassReference::Expression(class) => current = class,
                    class => break SpineFoot::Class(class),
                }
            }
            Expression::PartialApplication(PartialApplication::Method(application)) => {
                links.push(SpineLink::PartialMethod(application));
                current = application.object;
            }
            Expression::PartialApplication(PartialApplication::StaticMethod(application)) => {
                links.push(SpineLink::PartialStaticMethod(application));
                match &application.class {
                    ClassReference::Expression(class) => current = class,
                    class => break SpineFoot::Class(class),
                }
            }
            other => break SpineFoot::Expression(other),
        }
    }
}

fn render_spine<'source, 'arena, A>(
    start: Span,
    foot: SpineFoot<'source, 'arena>,
    links: Vec<SpineLink<'source, 'arena>>,
    f: &mut FormatterState<'arena, A>,
) -> Document<'arena, A>
where
    A: Arena,
{
    let break_member_chain = should_break_member_chain(&links);
    let has_logical_operator = links
        .iter()
        .any(|link| matches!(link, SpineLink::Binary(binary) if binary.operator.is_logical()));
    let mut parts = f.vec();
    let mut segment = f.vec();
    let head = match foot {
        SpineFoot::Expression(expression)
            if links.last().is_some_and(SpineLink::is_instance_member)
                && matches!(expression.unparenthesized(), Expression::Instantiation(_)) =>
        {
            expression.unparenthesized().format(f)
        }
        SpineFoot::Expression(expression) => expression.format(f),
        SpineFoot::Callee(Callee::Identifier(identifier)) => f.text(identifier.value()),
        SpineFoot::Callee(Callee::Expression(expression)) => expression.format(f),
        SpineFoot::Class(class) => f.format_class_reference(class),
    };
    segment.push(head);

    let last = links.len() - 1;
    for (index, link) in links.into_iter().rev().enumerate() {
        let end = match link {
            SpineLink::Binary(binary) => binary.rhs.span(),
            SpineLink::Index(access) => access.right_bracket,
            SpineLink::Property(access) => access.property.span(),
            SpineLink::NullSafeProperty(access) => access.property.span(),
            SpineLink::Method(call) => call.argument_list.span(),
            SpineLink::NullSafeMethod(call) => call.argument_list.span(),
            SpineLink::Postfix(postfix) => postfix.operator.span(),
            SpineLink::TypeCheck(operation) => operation.r#type.span(),
            SpineLink::Call(call) => call.argument_list.span(),
            SpineLink::PartialCall(application) => application.argument_list.span(),
            SpineLink::StaticProperty(access) => access.property.span(),
            SpineLink::ClassConstant(access) => access.constant.span(),
            SpineLink::StaticMethod(call) => call.argument_list.span(),
            SpineLink::PartialMethod(application) => application.argument_list.span(),
            SpineLink::PartialStaticMethod(application) => application.argument_list.span(),
        };
        match link {
            SpineLink::Binary(binary) => {
                if has_logical_operator && binary.operator.is_logical() {
                    parts.push(Document::Group(
                        Group::new(segment).with_break_mode(BreakMode::Independent),
                    ));
                    parts.push(f.line());
                    parts.push(f.text(binary.operator.as_str()));
                    parts.push(f.space());
                    segment = f.vec();
                } else {
                    segment.push(
                        if has_logical_operator || should_inline_binary_rhs(binary) {
                            f.space()
                        } else {
                            f.line()
                        },
                    );
                    segment.push(f.text(binary.operator.as_str()));
                    segment.push(f.space());
                }
                let rhs = binary.rhs.format(f);
                segment.push(rhs);
            }
            SpineLink::Index(access) => {
                segment.push(f.text("["));
                let index = access.index.format(f);
                segment.push(index);
                segment.push(f.text("]"));
            }
            SpineLink::Property(access) => {
                segment.push(member_link(
                    f,
                    break_member_chain,
                    [f.text("->"), f.text(access.property.value)],
                ));
            }
            SpineLink::NullSafeProperty(access) => {
                segment.push(member_link(
                    f,
                    break_member_chain,
                    [f.text("?->"), f.text(access.property.value)],
                ));
            }
            SpineLink::Method(call) => {
                let turbofish = f.format_optional_turbofish(call.type_arguments.as_ref());
                let arguments = f.format_argument_list(&call.argument_list);
                segment.push(member_link(
                    f,
                    break_member_chain,
                    [
                        f.text("->"),
                        f.text(call.method.value),
                        turbofish,
                        arguments,
                    ],
                ));
            }
            SpineLink::NullSafeMethod(call) => {
                let turbofish = f.format_optional_turbofish(call.type_arguments.as_ref());
                let arguments = f.format_argument_list(&call.argument_list);
                segment.push(member_link(
                    f,
                    break_member_chain,
                    [
                        f.text("?->"),
                        f.text(call.method.value),
                        turbofish,
                        arguments,
                    ],
                ));
            }
            SpineLink::Postfix(postfix) => {
                segment.push(f.text(postfix.operator.as_str()));
            }
            SpineLink::TypeCheck(operation) => {
                let mut tail = f.vec();
                tail.push(if matches!(operation.operator, TypeOperator::Check(_)) {
                    f.space()
                } else {
                    f.line()
                });
                tail.push(f.text(type_operator_text(operation.operator)));
                tail.push(f.space());
                let r#type = operation.r#type.format(f);
                tail.push(r#type);
                segment.push(f.indent(tail));
            }
            SpineLink::Call(call) => {
                let turbofish = f.format_optional_turbofish(call.type_arguments.as_ref());
                segment.push(turbofish);
                let arguments = f.format_argument_list(&call.argument_list);
                segment.push(arguments);
            }
            SpineLink::PartialCall(application) => {
                let turbofish = f.format_optional_turbofish(application.type_arguments.as_ref());
                segment.push(turbofish);
                let arguments = f.format_partial_argument_list(&application.argument_list);
                segment.push(arguments);
            }
            SpineLink::StaticProperty(access) => {
                segment.push(f.concat([f.text("::"), f.text(access.property.name)]));
            }
            SpineLink::ClassConstant(access) => {
                segment.push(f.concat([f.text("::"), f.text(access.constant.value)]));
            }
            SpineLink::StaticMethod(call) => {
                let turbofish = f.format_optional_turbofish(call.type_arguments.as_ref());
                let arguments = f.format_argument_list(&call.argument_list);
                segment.push(f.concat([
                    f.text("::"),
                    f.text(call.method.value),
                    turbofish,
                    arguments,
                ]));
            }
            SpineLink::PartialMethod(application) => {
                let turbofish = f.format_optional_turbofish(application.type_arguments.as_ref());
                let arguments = f.format_partial_argument_list(&application.argument_list);
                segment.push(member_link(
                    f,
                    break_member_chain,
                    [
                        f.text("->"),
                        f.text(application.method.value),
                        turbofish,
                        arguments,
                    ],
                ));
            }
            SpineLink::PartialStaticMethod(application) => {
                let turbofish = f.format_optional_turbofish(application.type_arguments.as_ref());
                let arguments = f.format_partial_argument_list(&application.argument_list);
                segment.push(f.concat([
                    f.text("::"),
                    f.text(application.method.value),
                    turbofish,
                    arguments,
                ]));
            }
        }

        if index != last
            && let Some(trailing) = f.print_trailing_comments(start.join(end))
        {
            segment.push(trailing);
        }
    }

    if has_logical_operator {
        parts.push(Document::Group(
            Group::new(segment).with_break_mode(BreakMode::Independent),
        ));
        Document::Group(Group::new(parts).with_break_mode(BreakMode::Parent))
    } else {
        Document::Group(Group::new(segment))
    }
}

fn should_inline_binary_rhs(binary: &Binary<'_>) -> bool {
    if !binary.operator.is_comparison()
        && !matches!(binary.operator, BinaryOperator::NullCoalesce(_))
    {
        return false;
    }

    matches!(
        binary.rhs.unparenthesized(),
        Expression::Assignment(_)
            | Expression::Vec(_)
            | Expression::Dict(_)
            | Expression::Tuple(_)
            | Expression::Call(_)
            | Expression::Closure(_)
            | Expression::ShortClosure(_)
            | Expression::Match(_)
            | Expression::Instantiation(_)
    )
}

impl SpineLink<'_, '_> {
    const fn is_instance_member(&self) -> bool {
        matches!(
            self,
            Self::Property(_)
                | Self::NullSafeProperty(_)
                | Self::Method(_)
                | Self::NullSafeMethod(_)
                | Self::PartialMethod(_)
        )
    }

    const fn is_instance_method(&self) -> bool {
        matches!(
            self,
            Self::Method(_) | Self::NullSafeMethod(_) | Self::PartialMethod(_)
        )
    }
}

fn should_break_member_chain(links: &[SpineLink<'_, '_>]) -> bool {
    let members = links
        .iter()
        .filter(|link| link.is_instance_member())
        .count();
    let methods = links
        .iter()
        .filter(|link| link.is_instance_method())
        .count();

    methods >= 3 || members >= 4
}

fn member_link<'arena, A>(
    f: &FormatterState<'arena, A>,
    breakable: bool,
    documents: impl IntoIterator<Item = Document<'arena, A>>,
) -> Document<'arena, A>
where
    A: Arena,
{
    let mut contents = f.vec();
    if !breakable {
        contents.extend(documents);
        return Document::Array(contents);
    }

    contents.push(f.soft_line());
    contents.extend(documents);
    f.indent_if_break(contents)
}
