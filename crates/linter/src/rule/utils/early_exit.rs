use annotate_snippets::AnnotationKind;
use annotate_snippets::Level;
use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::arena::Arena;
use whim_syn::cst::control_flow::If;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::statement::Statement;

use crate::context::LintContext;
use crate::rule_meta::RuleMeta;

pub(crate) struct EarlyExitPattern {
    pub(crate) title: &'static str,
    pub(crate) primary: &'static str,
    pub(crate) context: &'static str,
    pub(crate) help: &'static str,
}

pub(crate) fn check_early_exit<A: Arena>(
    ctx: &mut LintContext<'_, '_, A>,
    meta: &'static RuleMeta,
    level: Level<'static>,
    statement: &If<'_>,
    subject: Span,
    max_allowed_statements: usize,
    pattern: &EarlyExitPattern,
) {
    if statement.r#else.is_some() {
        return;
    }

    let body = statement.body.statements;
    if body.len() <= max_allowed_statements
        || (body.len() == 1 && is_early_exit_statement(&body[0]))
    {
        return;
    }

    ctx.report(
        meta,
        level,
        (statement.span(), pattern.primary),
        pattern.title,
        [AnnotationKind::Context
            .span(subject.into())
            .label(pattern.context)],
        [
            Level::NOTE
                .message("An early exit reduces the nesting around the main path.")
                .into(),
            Level::HELP.message(pattern.help).into(),
        ],
    );
}

pub(crate) fn extract_single_if<'a>(statements: &'a [Statement<'a>]) -> Option<&'a If<'a>> {
    let [Statement::If(statement)] = statements else {
        return None;
    };

    Some(statement)
}

fn is_early_exit_statement(statement: &Statement<'_>) -> bool {
    matches!(
        statement,
        Statement::Expression(expression)
            if matches!(
                expression.expression.unparenthesized(),
                Expression::Return(_)
                    | Expression::Throw(_)
                    | Expression::Break(_)
                    | Expression::Continue(_)
            )
    )
}
