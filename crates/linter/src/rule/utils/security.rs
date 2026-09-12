use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::cst::access::Access;
use whim_syn::cst::atom::Literal;
use whim_syn::cst::atom::LiteralString;
use whim_syn::cst::call::Call;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::operation::AssignmentTarget;

pub(crate) fn get_password(expression: &Expression<'_>) -> Option<Span> {
    match expression.unparenthesized() {
        Expression::Literal(Literal::String(literal)) if is_password_literal(literal) => {
            Some(literal.span())
        }
        Expression::Assignment(assignment) => {
            get_password_from_target(&assignment.target).or_else(|| get_password(assignment.value))
        }
        Expression::ArrayAccess(access) => get_password(access.index),
        Expression::Variable(variable) if is_password(variable.name.as_bytes()) => {
            Some(variable.span())
        }
        Expression::Access(access) => {
            let (name, span) = match access {
                Access::Constant(access) => (access.name.value(), access.name.span()),
                Access::Property(access) => (access.property.value, access.property.span()),
                Access::NullSafeProperty(access) => (access.property.value, access.property.span()),
                Access::StaticProperty(access) => (access.property.name, access.property.span()),
                Access::ClassConstant(access) => (access.constant.value, access.constant.span()),
            };

            is_password(name.as_bytes()).then_some(span)
        }
        Expression::Call(call) => {
            let method = match call {
                Call::Method(call) => &call.method,
                Call::NullSafeMethod(call) => &call.method,
                Call::StaticMethod(call) => &call.method,
                Call::Function(_) => return None,
            };

            is_password(method.value.as_bytes()).then_some(method.span())
        }
        _ => None,
    }
}

pub(crate) fn get_password_from_target(target: &AssignmentTarget<'_>) -> Option<Span> {
    let (name, span) = match target {
        AssignmentTarget::Variable(variable) => (variable.name, variable.span()),
        AssignmentTarget::Property(access) => (access.property.value, access.property.span()),
        AssignmentTarget::StaticProperty(access) => (access.property.name, access.property.span()),
        AssignmentTarget::ArrayIndex(access) => return get_password(access.index),
        _ => return None,
    };

    is_password(name.as_bytes()).then_some(span)
}

pub(crate) fn is_password_literal(literal: &LiteralString<'_>) -> bool {
    is_password(&literal.raw.as_bytes()[1..literal.raw.len() - 1])
}

pub(crate) fn is_password(name: &[u8]) -> bool {
    !name.starts_with(b"--")
        && [
            b"password".as_slice(),
            b"token",
            b"secret",
            b"apikey",
            b"api_key",
        ]
        .iter()
        .any(|suffix| {
            name.len() >= suffix.len()
                && name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
        })
}
