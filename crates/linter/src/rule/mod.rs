use annotate_snippets::Level;
#[cfg(feature = "serde")]
use serde::de::DeserializeOwned;

use whim_syn::arena::Arena;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;

use crate::context::LintContext;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;
use crate::settings::Settings;

pub mod best_practices;
pub mod clarity;
pub mod correctness;
pub mod maintainability;
pub mod redundancy;
pub mod security;
mod utils;

#[cfg(test)]
pub mod tests;
pub use best_practices::LoopDoesNotIterateConfig;
pub use best_practices::LoopDoesNotIterateRule;
pub use best_practices::NoParameterShadowingConfig;
pub use best_practices::NoParameterShadowingRule;
pub use best_practices::PreferEarlyContinueConfig;
pub use best_practices::PreferEarlyContinueRule;
pub use best_practices::PreferEarlyReturnConfig;
pub use best_practices::PreferEarlyReturnRule;
pub use best_practices::PreferWhileLoopConfig;
pub use best_practices::PreferWhileLoopRule;
pub use best_practices::UseCompoundAssignmentConfig;
pub use best_practices::UseCompoundAssignmentRule;
pub use best_practices::YodaConditionsConfig;
pub use best_practices::YodaConditionsRule;
pub use clarity::NoMultiAssignmentsConfig;
pub use clarity::NoMultiAssignmentsRule;
pub use clarity::ReadableLiteralConfig;
pub use clarity::ReadableLiteralRule;
pub use clarity::TaggedFixmeConfig;
pub use clarity::TaggedFixmeRule;
pub use clarity::TaggedTodoConfig;
pub use clarity::TaggedTodoRule;
pub use correctness::NoAssignInArgumentConfig;
pub use correctness::NoAssignInArgumentRule;
pub use correctness::NoAssignInConditionConfig;
pub use correctness::NoAssignInConditionRule;
pub use correctness::NoDeadStoreConfig;
pub use correctness::NoDeadStoreRule;
pub use correctness::NoEmptyCatchClauseConfig;
pub use correctness::NoEmptyCatchClauseRule;
pub use correctness::NoRedundantVariableConfig;
pub use correctness::NoRedundantVariableRule;
pub use maintainability::CyclomaticComplexityConfig;
pub use maintainability::CyclomaticComplexityRule;
pub use maintainability::ExcessiveNestingConfig;
pub use maintainability::ExcessiveNestingRule;
pub use redundancy::ConstantConditionConfig;
pub use redundancy::ConstantConditionRule;
pub use redundancy::InlineVariableReturnConfig;
pub use redundancy::InlineVariableReturnRule;
pub use redundancy::NoEmptyCommentConfig;
pub use redundancy::NoEmptyCommentRule;
pub use redundancy::NoRedundantContinueConfig;
pub use redundancy::NoRedundantContinueRule;
pub use redundancy::NoRedundantElseConfig;
pub use redundancy::NoRedundantElseRule;
pub use redundancy::NoRedundantFinalConfig;
pub use redundancy::NoRedundantFinalRule;
pub use redundancy::NoRedundantNullsafeConfig;
pub use redundancy::NoRedundantNullsafeRule;
pub use redundancy::NoRedundantReadonlyConfig;
pub use redundancy::NoRedundantReadonlyRule;
pub use redundancy::NoRedundantStaticConfig;
pub use redundancy::NoRedundantStaticRule;
pub use redundancy::NoRedundantStringConcatConfig;
pub use redundancy::NoRedundantStringConcatRule;
pub use redundancy::NoRedundantUseConfig;
pub use redundancy::NoRedundantUseRule;
pub use redundancy::NoSelfAssignmentConfig;
pub use redundancy::NoSelfAssignmentRule;
pub use security::DisallowedSymbol;
pub use security::DisallowedSymbolsConfig;
pub use security::DisallowedSymbolsRule;
pub use security::NoDebugSymbolsConfig;
pub use security::NoDebugSymbolsRule;
pub use security::NoInsecureComparisonConfig;
pub use security::NoInsecureComparisonRule;
pub use security::NoLiteralPasswordConfig;
pub use security::NoLiteralPasswordRule;
pub use security::SensitiveParameterConfig;
pub use security::SensitiveParameterRule;

#[cfg(feature = "serde")]
pub trait Config: Default + DeserializeOwned {
    fn default_enabled() -> bool {
        true
    }
    fn level(&self) -> Level<'static>;
}

#[cfg(not(feature = "serde"))]
pub trait Config: Default {
    fn default_enabled() -> bool {
        true
    }

    fn level(&self) -> Level<'static>;
}

pub trait LintRule {
    type Config: Config;
    fn meta() -> &'static RuleMeta;
    fn targets() -> &'static [NodeKind];
    fn build(settings: &RuleSettings<Self::Config>) -> Self;
    fn check<'arena, A: Arena>(&self, ctx: &mut LintContext<'_, 'arena, A>, node: Node<'_, 'arena>);
}

macro_rules! define_rules {
    ($($variant:ident($module:ident @ $rule:ident)),* $(,)?) => {
        #[derive(Debug, Clone)]
        pub enum AnyRule { $($variant($rule)),* }

        impl AnyRule {
            pub fn get_all_for(settings: &Settings, only: Option<&[String]>, include_disabled: bool) -> Vec<(Self, Vec<String>)> {
                let mut rules = Vec::new();
                $(
                    let enabled = if let Some(only) = only {
                        only.iter().any(|code| code == $rule::meta().code)
                    } else { include_disabled || settings.rules.$module.is_enabled() };
                    if enabled {
                        rules.push((Self::$variant($rule::build(&settings.rules.$module)), settings.rules.$module.exclude.clone()));
                    }
                )*

                rules
            }

            pub fn name(&self) -> &'static str { self.meta().name }

            pub fn code(&self) -> &'static str { self.meta().code }

            pub fn default_level(&self) -> Level<'static> {
                match self { $(Self::$variant(_) => <$rule as LintRule>::Config::default().level()),* }
            }

            pub fn default_enabled(&self) -> bool {
                match self { $(Self::$variant(_) => <$rule as LintRule>::Config::default_enabled()),* }
            }

            pub fn meta(&self) -> &'static RuleMeta {
                match self { $(Self::$variant(_) => $rule::meta()),* }
            }

            pub fn targets(&self) -> &'static [NodeKind] {
                match self { $(Self::$variant(_) => $rule::targets()),* }
            }

            pub fn check<'arena, A: Arena>(&self, ctx: &mut LintContext<'_, 'arena, A>, node: Node<'_, 'arena>) {
                match self { $(Self::$variant(rule) => rule.check(ctx, node)),* }
            }
        }
    };
}

define_rules! {
    TaggedTodo(tagged_todo @ TaggedTodoRule),
    TaggedFixme(tagged_fixme @ TaggedFixmeRule),
    LoopDoesNotIterate(loop_does_not_iterate @ LoopDoesNotIterateRule),
    YodaConditions(yoda_conditions @ YodaConditionsRule),
    UseCompoundAssignment(use_compound_assignment @ UseCompoundAssignmentRule),
    NoAssignInArgument(no_assign_in_argument @ NoAssignInArgumentRule),
    NoAssignInCondition(no_assign_in_condition @ NoAssignInConditionRule),
    NoDeadStore(no_dead_store @ NoDeadStoreRule),
    ExcessiveNesting(excessive_nesting @ ExcessiveNestingRule),
    NoRedundantStatic(no_redundant_static @ NoRedundantStaticRule),
    NoRedundantFinal(no_redundant_final @ NoRedundantFinalRule),
    NoRedundantElse(no_redundant_else @ NoRedundantElseRule),
    NoRedundantContinue(no_redundant_continue @ NoRedundantContinueRule),
    NoLiteralPassword(no_literal_password @ NoLiteralPasswordRule),
    NoInsecureComparison(no_insecure_comparison @ NoInsecureComparisonRule),
    NoRedundantVariable(no_redundant_variable @ NoRedundantVariableRule),
    NoSelfAssignment(no_self_assignment @ NoSelfAssignmentRule),
    NoParameterShadowing(no_parameter_shadowing @ NoParameterShadowingRule),
    NoRedundantUse(no_redundant_use @ NoRedundantUseRule),
    SensitiveParameter(sensitive_parameter @ SensitiveParameterRule),
    NoDebugSymbols(no_debug_symbols @ NoDebugSymbolsRule),
    NoEmptyCatchClause(no_empty_catch_clause @ NoEmptyCatchClauseRule),
    NoRedundantReadonly(no_redundant_readonly @ NoRedundantReadonlyRule),
    NoRedundantNullsafe(no_redundant_nullsafe @ NoRedundantNullsafeRule),
    ConstantCondition(constant_condition @ ConstantConditionRule),
    InlineVariableReturn(inline_variable_return @ InlineVariableReturnRule),
    NoMultiAssignments(no_multi_assignments @ NoMultiAssignmentsRule),
    PreferWhileLoop(prefer_while_loop @ PreferWhileLoopRule),
    PreferEarlyReturn(prefer_early_return @ PreferEarlyReturnRule),
    PreferEarlyContinue(prefer_early_continue @ PreferEarlyContinueRule),
    ReadableLiteral(readable_literal @ ReadableLiteralRule),
    NoRedundantStringConcat(no_redundant_string_concat @ NoRedundantStringConcatRule),
    NoEmptyComment(no_empty_comment @ NoEmptyCommentRule),
    CyclomaticComplexity(cyclomatic_complexity @ CyclomaticComplexityRule),
    DisallowedSymbols(disallowed_symbols @ DisallowedSymbolsRule),
}

#[macro_export]
macro_rules! test_lint_success {
    {
        name = $test_name:ident,
        rule = $rule:ty,
        settings = $settings:expr,
        code = $code:expr $(,)?
    } => {
        #[test]
        fn $test_name() {
            $crate::rule::tests::run_lint_test::<$rule, _>($code, Some(0), Some($settings));
        }
    };
    {
        name = $test_name:ident,
        rule = $rule:ty,
        code = $code:expr $(,)?
    } => {
        #[test]
        fn $test_name() {
            $crate::rule::tests::run_lint_test::<$rule, fn(&mut $crate::settings::Settings)>(
                $code,
                Some(0),
                None,
            );
        }
    };
}

#[macro_export]
macro_rules! test_lint_failure {
    {
        name = $test_name:ident,
        rule = $rule:ty,
        $(count = $count:expr,)?
        settings = $settings:expr,
        code = $code:expr $(,)?
    } => {
        #[test]
        fn $test_name() {
            $crate::rule::tests::run_lint_test::<$rule, _>(
                $code,
                None $(.or(Some($count)))?,
                Some($settings),
            );
        }
    };
    {
        name = $test_name:ident,
        rule = $rule:ty,
        $(count = $count:expr,)?
        code = $code:expr $(,)?
    } => {
        #[test]
        fn $test_name() {
            $crate::rule::tests::run_lint_test::<$rule, fn(&mut $crate::settings::Settings)>(
                $code,
                None $(.or(Some($count)))?,
                None,
            );
        }
    };
}
