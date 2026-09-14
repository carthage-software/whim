use std::mem::replace;

use annotate_snippets::AnnotationKind;
use annotate_snippets::Level;

use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::arena::Arena;
use whim_syn::arena::Vec as ArenaVec;
use whim_syn::cst::atom::Literal;
use whim_syn::cst::call::Argument;
use whim_syn::cst::declaration::Attribute;
use whim_syn::cst::declaration::AttributeList;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::node::Node;
use whim_syn::cst::operation::BinaryOperator;
use whim_syn::cst::statement::TopLevelStatement;

use crate::context::LintContext;
use crate::rule::AnyRule;
use crate::rule::utils::names::Resolver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Control {
    Allow,
    Warn,
    Deny,
    Forbid,
}

impl Control {
    pub(crate) fn level(self) -> Option<Level<'static>> {
        match self {
            Self::Allow => None,
            Self::Warn => Some(Level::WARNING),
            Self::Deny | Self::Forbid => Some(Level::ERROR),
        }
    }
}

struct Setting {
    code: &'static str,
    control: Control,
    span: Span,
}

struct AttributeScope {
    span: Span,
    parent: Option<usize>,
    settings: Vec<Setting>,
}

#[derive(Default)]
pub(crate) struct AttributeScopes {
    scopes: Vec<AttributeScope>,
}

impl AttributeScopes {
    pub(crate) fn collect<A: Arena>(ctx: &mut LintContext<'_, '_, A>) -> Self {
        let mut scopes = Self::default();
        if ctx.source.contains("#![") {
            let mut settings = Vec::new();
            scopes.collect_file_attributes(
                ctx,
                ctx.program.statements,
                Resolver::default(),
                &mut settings,
            );

            if !settings.is_empty() {
                scopes.scopes.push(AttributeScope {
                    span: ctx.program.span(),
                    parent: None,
                    settings,
                });
            }
        }

        if !ctx.source.contains("#[") {
            return scopes;
        }

        enum Step<'ast, 'arena> {
            Enter(Node<'ast, 'arena>),
            ExitScope(Option<usize>),
            ExitNamespace(Resolver),
        }

        let mut resolver = Resolver::default();
        let mut parent = scopes.scopes.len().checked_sub(1);
        let mut stack = ArenaVec::with_capacity_in(64, ctx.arena);
        stack.push(Step::Enter(Node::Program(ctx.program)));
        while let Some(step) = stack.pop() {
            match step {
                Step::ExitScope(previous) => parent = previous,
                Step::ExitNamespace(previous) => resolver = previous,
                Step::Enter(node) => {
                    match node {
                        Node::Namespace(namespace) => {
                            stack.push(Step::ExitNamespace(replace(
                                &mut resolver,
                                Resolver::for_namespace(namespace.name.value()),
                            )));
                        }
                        Node::Use(declaration) => resolver.collect_use(declaration),
                        _ => {}
                    }

                    let mut settings: Vec<Setting> = Vec::new();
                    for list in attribute_lists(node) {
                        for attribute in &list.attributes {
                            scopes.collect_attribute(
                                ctx,
                                &resolver,
                                attribute,
                                parent,
                                &mut settings,
                            );
                        }
                    }

                    if !settings.is_empty() {
                        stack.push(Step::ExitScope(parent));
                        let index = scopes.scopes.len();
                        scopes.scopes.push(AttributeScope {
                            span: node.span(),
                            parent,
                            settings,
                        });

                        parent = Some(index);
                    }

                    let start = stack.len();
                    node.visit_children(&mut |child| stack.push(Step::Enter(child)));
                    stack[start..].reverse();
                }
            }
        }

        scopes
    }

    pub(crate) fn get(&self, code: &str, span: Span) -> Option<Control> {
        let mut index = self
            .scopes
            .partition_point(|scope| scope.span.start <= span.start)
            .checked_sub(1);
        while let Some(current) = index {
            let scope = &self.scopes[current];
            if scope.span.start <= span.start && span.end <= scope.span.end {
                return self.setting(code, index).map(|setting| setting.control);
            }

            index = scope.parent;
        }

        None
    }

    pub(crate) fn enables(&self, code: &str) -> bool {
        self.scopes.iter().any(|scope| {
            scope
                .settings
                .iter()
                .any(|setting| setting.code == code && setting.control != Control::Allow)
        })
    }

    fn setting(&self, code: &str, mut index: Option<usize>) -> Option<&Setting> {
        while let Some(current) = index {
            let scope = &self.scopes[current];
            if let Some(setting) = scope
                .settings
                .iter()
                .rev()
                .find(|setting| setting.code == code)
            {
                return Some(setting);
            }

            index = scope.parent;
        }

        None
    }

    fn collect_file_attributes<A: Arena>(
        &self,
        ctx: &mut LintContext<'_, '_, A>,
        statements: &[TopLevelStatement<'_>],
        mut resolver: Resolver,
        settings: &mut Vec<Setting>,
    ) {
        for statement in statements {
            match statement {
                TopLevelStatement::FileAttributeList(list) => {
                    for attribute in &list.attributes {
                        self.collect_attribute(ctx, &resolver, attribute, None, settings);
                    }
                }
                TopLevelStatement::Namespace(namespace) => self.collect_file_attributes(
                    ctx,
                    namespace.statements(),
                    Resolver::for_namespace(namespace.name.value()),
                    settings,
                ),
                TopLevelStatement::Use(declaration) => resolver.collect_use(declaration),
                _ => {}
            }
        }
    }

    fn collect_attribute<A: Arena>(
        &self,
        ctx: &mut LintContext<'_, '_, A>,
        resolver: &Resolver,
        attribute: &Attribute<'_>,
        parent: Option<usize>,
        settings: &mut Vec<Setting>,
    ) {
        let control = match resolver.resolve(&attribute.name).as_str() {
            "Whim\\Lint\\Allow" => Control::Allow,
            "Whim\\Lint\\Warn" => Control::Warn,
            "Whim\\Lint\\Deny" => Control::Deny,
            "Whim\\Lint\\Forbid" => Control::Forbid,
            _ => return,
        };

        let Some(code) = rule_name(ctx, attribute) else {
            return;
        };

        let previous = settings
            .iter()
            .rev()
            .find(|entry| entry.code == code)
            .or_else(|| self.setting(code, parent));

        if let Some(previous) = previous
            && previous.control == Control::Forbid
        {
            if matches!(control, Control::Allow | Control::Warn) {
                ctx.attribute_error(
                    attribute.span(),
                    format!("Cannot lower the level of forbidden lint `{code}`."),
                    [AnnotationKind::Context
                        .span(previous.span.into())
                        .label("this attribute forbids the lint")],
                    "Remove the attribute that lowers the level, or change the enclosing `Forbid`.",
                );
            }

            return;
        }

        settings.push(Setting {
            code,
            control,
            span: attribute.span(),
        });
    }
}

fn attribute_lists<'arena>(node: Node<'_, 'arena>) -> &'arena [AttributeList<'arena>] {
    match node {
        Node::Class(node) => node.attribute_lists,
        Node::Interface(node) => node.attribute_lists,
        Node::Enum(node) => node.attribute_lists,
        Node::Function(node) => node.attribute_lists,
        Node::Closure(node) => node.attribute_lists,
        Node::Method(node) => node.attribute_lists,
        Node::Property(node) => node.attribute_lists,
        Node::Parameter(node) => node.attribute_lists,
        Node::ClassLikeConstant(node) => node.attribute_lists,
        Node::EnumCase(node) => node.attribute_lists,
        Node::Constant(node) => node.attribute_lists,
        Node::TypeAlias(node) => node.attribute_lists,
        Node::Newtype(node) => node.attribute_lists,
        _ => &[],
    }
}

fn rule_name<A: Arena>(
    ctx: &mut LintContext<'_, '_, A>,
    attribute: &Attribute<'_>,
) -> Option<&'static str> {
    let expression = attribute.argument_list.as_ref().and_then(|list| {
        let mut parameters = [None; 2];
        let mut positional = 0;
        for argument in &list.arguments {
            let index = match argument {
                Argument::Positional(_) => {
                    let index = positional;
                    positional += 1;
                    index
                }
                Argument::Named(argument) => match argument.name.value {
                    "rule" => 0,
                    "reason" => 1,
                    _ => return None,
                },
            };

            if parameters
                .get_mut(index)?
                .replace(argument.value())
                .is_some()
            {
                return None;
            }
        }
        parameters[0]
    });

    let Some(expression) = expression else {
        ctx.attribute_error(
            attribute.span(),
            "A lint attribute requires `rule` and accepts an optional `reason`.",
            [],
            "Pass each parameter once, using positional arguments or the names `rule` and `reason`.",
        );

        return None;
    };

    let code = match evaluate_string(expression) {
        Ok(code) => code,
        Err(span) => {
            ctx.attribute_error(
                span,
                "The linter cannot determine this rule name.",
                [],
                "Use a string literal, optionally joined with `.` or wrapped in parentheses. The linter does not resolve constants or run code.",
            );

            return None;
        }
    };

    match AnyRule::meta_for_code(&code) {
        Some(meta) => Some(meta.code),
        None => {
            ctx.attribute_error(
                expression.span(),
                format!("Unknown lint rule {code:?}."),
                [],
                "Use an exact rule code from the linting guide, such as `sensitive-parameter`.",
            );

            None
        }
    }
}

fn evaluate_string(expression: &Expression<'_>) -> Result<String, Span> {
    let mut bytes = Vec::new();
    let mut stack = vec![expression];
    while let Some(part) = stack.pop() {
        match part {
            Expression::Literal(Literal::String(literal)) => bytes.extend_from_slice(literal.value),
            Expression::Parenthesized(parenthesized) => stack.push(parenthesized.expression),
            Expression::Binary(binary)
                if matches!(binary.operator, BinaryOperator::StringConcat(_)) =>
            {
                stack.push(binary.rhs);
                stack.push(binary.lhs);
            }
            _ => return Err(part.span()),
        }
    }

    String::from_utf8(bytes).map_err(|_| expression.span())
}

#[cfg(test)]
mod tests;
