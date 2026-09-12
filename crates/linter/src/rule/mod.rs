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
pub use best_practices::LoopDoesNotIterateConfig;
pub use best_practices::LoopDoesNotIterateRule;
pub use best_practices::UseCompoundAssignmentConfig;
pub use best_practices::UseCompoundAssignmentRule;
pub use best_practices::YodaConditionsConfig;
pub use best_practices::YodaConditionsRule;
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
pub use maintainability::ExcessiveNestingConfig;
pub use maintainability::ExcessiveNestingRule;
pub use redundancy::NoRedundantContinueConfig;
pub use redundancy::NoRedundantContinueRule;
pub use redundancy::NoRedundantElseConfig;
pub use redundancy::NoRedundantElseRule;
pub use redundancy::NoRedundantFinalConfig;
pub use redundancy::NoRedundantFinalRule;
pub use redundancy::NoRedundantStaticConfig;
pub use redundancy::NoRedundantStaticRule;
pub use security::NoInsecureComparisonConfig;
pub use security::NoInsecureComparisonRule;
pub use security::NoLiteralPasswordConfig;
pub use security::NoLiteralPasswordRule;

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
}
