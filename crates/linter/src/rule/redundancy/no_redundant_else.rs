use annotate_snippets::AnnotationKind;
use annotate_snippets::Level;
use indoc::indoc;

use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::call::Call;
use whim_syn::cst::construct::Construct;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::operation::AssignmentOperator;
use whim_syn::cst::statement::Statement;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct NoRedundantElseRule {
    meta: &'static RuleMeta,
    cfg: NoRedundantElseConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoRedundantElseConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for NoRedundantElseConfig {
    fn default() -> Self {
        Self { level: Level::HELP }
    }
}

impl Config for NoRedundantElseConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoRedundantElseRule {
    type Config = NoRedundantElseConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Redundant Else",
            code: "no-redundant-else",
            description: indoc! {"
                Flags `if`/`else` statements where the `if` branch always terminates
                control flow (via `return`, `throw`, `exit!`, `panic!`, `continue`, or `break`).

                When the `if` branch unconditionally terminates, the `else` branch becomes
                unnecessary nesting. Extracting the `else` body to follow the `if` flattens
                the control flow without changing semantics.
            "},
            good_example: indoc! {r#"
                function process($user) {
                    if (!$user->isVerified()) {
                        return;
                    }

                    $user->login();
                }
            "#},
            bad_example: indoc! {r#"
                function process($user) {
                    if (!$user->isVerified()) {
                        return;
                    } else {
                        $user->login();
                    }
                }
            "#},
            category: Category::Redundancy,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::If]
    }

    fn build(settings: &RuleSettings<Self::Config>) -> Self {
        Self {
            meta: Self::meta(),
            cfg: settings.config.clone(),
        }
    }

    fn check<'arena, A: Arena>(
        &self,
        ctx: &mut LintContext<'_, 'arena, A>,
        node: Node<'_, 'arena>,
    ) {
        let Node::If(statement) = node else {
            return;
        };

        let Some(otherwise) = &statement.r#else else {
            return;
        };

        let Some(Statement::Expression(last)) = statement.body.statements.last() else {
            return;
        };

        if !expression_always_terminates(last.expression) {
            return;
        }

        ctx.report(
            self.meta,
            self.cfg.level(),
            (otherwise.r#else.span(), "this `else` adds needless nesting"),
            "Redundant `else` after a branch that always exits.",
            [AnnotationKind::Context.span(last.span().into()).label("the `if` branch always exits here")],
            [Level::HELP
                .message("Move the `else` body after the `if`. For `else if`, remove `else` to start a separate `if`.")
                .into()],
        );
    }
}

fn expression_always_terminates(expression: &Expression<'_>) -> bool {
    match expression.unparenthesized() {
        Expression::Return(_)
        | Expression::Break(_)
        | Expression::Continue(_)
        | Expression::Throw(_)
        | Expression::Construct(Construct::Exit(_) | Construct::Panic(_)) => true,
        Expression::UnaryPrefix(prefix) => expression_always_terminates(prefix.operand),
        Expression::UnaryPostfix(postfix) => expression_always_terminates(postfix.operand),
        Expression::Assignment(assignment) => {
            !matches!(
                assignment.operator,
                AssignmentOperator::Coalesce(_)
                    | AssignmentOperator::LogicalAnd(_)
                    | AssignmentOperator::LogicalOr(_)
            ) && expression_always_terminates(assignment.value)
        }
        Expression::Call(Call::NullSafeMethod(_)) => false,
        Expression::Call(call) => call
            .get_argument_list()
            .arguments
            .iter()
            .any(|argument| expression_always_terminates(argument.value())),
        Expression::Match(matching) => {
            expression_always_terminates(matching.expression)
                || matching
                    .arms
                    .iter()
                    .all(|arm| expression_always_terminates(arm.expression))
        }
        _ => false,
    }
}
