use crate::rule::Config;
use crate::rule::ExcessiveNestingConfig;
use crate::rule::LoopDoesNotIterateConfig;
use crate::rule::NoAssignInArgumentConfig;
use crate::rule::NoAssignInConditionConfig;
use crate::rule::NoDeadStoreConfig;
use crate::rule::NoInsecureComparisonConfig;
use crate::rule::NoLiteralPasswordConfig;
use crate::rule::NoRedundantContinueConfig;
use crate::rule::NoRedundantElseConfig;
use crate::rule::NoRedundantFinalConfig;
use crate::rule::NoRedundantStaticConfig;
use crate::rule::TaggedFixmeConfig;
use crate::rule::TaggedTodoConfig;
use crate::rule::UseCompoundAssignmentConfig;
use crate::rule::YodaConditionsConfig;

#[cfg(feature = "serde")]
pub mod level {
    use annotate_snippets::Level;
    use serde::Deserialize;
    use serde::Deserializer;
    use serde::Serializer;
    use serde::de::Error;
    use serde::ser::Error as SerializeError;

    pub fn serialize<S: Serializer>(
        level: &Level<'static>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let name = if *level == Level::ERROR {
            "error"
        } else if *level == Level::WARNING {
            "warning"
        } else if *level == Level::HELP {
            "help"
        } else if *level == Level::NOTE {
            "note"
        } else {
            return Err(S::Error::custom("unsupported diagnostic level"));
        };

        serializer.serialize_str(name)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Level<'static>, D::Error> {
        let name = String::deserialize(deserializer)?;
        match name.as_str() {
            "error" | "Error" | "err" => Ok(Level::ERROR),
            "warning" | "Warning" | "warn" => Ok(Level::WARNING),
            "help" | "Help" => Ok(Level::HELP),
            "note" | "Note" => Ok(Level::NOTE),
            _ => Err(D::Error::unknown_variant(
                &name,
                &["error", "warning", "help", "note"],
            )),
        }
    }
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default, deny_unknown_fields))]
pub struct Settings {
    pub rules: RulesSettings,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(
        default,
        deny_unknown_fields,
        bound = "C: serde::Serialize + serde::de::DeserializeOwned"
    )
)]
pub struct RuleSettings<C: Config> {
    pub enabled: bool,
    pub exclude: Vec<String>,
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub config: C,
}

impl<C: Config> Default for RuleSettings<C> {
    fn default() -> Self {
        Self {
            enabled: C::default_enabled(),
            exclude: Vec::new(),
            config: C::default(),
        }
    }
}

impl<C: Config> RuleSettings<C> {
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct RulesSettings {
    pub tagged_todo: RuleSettings<TaggedTodoConfig>,
    pub tagged_fixme: RuleSettings<TaggedFixmeConfig>,
    pub loop_does_not_iterate: RuleSettings<LoopDoesNotIterateConfig>,
    pub yoda_conditions: RuleSettings<YodaConditionsConfig>,
    pub use_compound_assignment: RuleSettings<UseCompoundAssignmentConfig>,
    pub no_assign_in_argument: RuleSettings<NoAssignInArgumentConfig>,
    pub no_assign_in_condition: RuleSettings<NoAssignInConditionConfig>,
    pub no_dead_store: RuleSettings<NoDeadStoreConfig>,
    pub excessive_nesting: RuleSettings<ExcessiveNestingConfig>,
    pub no_redundant_static: RuleSettings<NoRedundantStaticConfig>,
    pub no_redundant_final: RuleSettings<NoRedundantFinalConfig>,
    pub no_redundant_else: RuleSettings<NoRedundantElseConfig>,
    pub no_redundant_continue: RuleSettings<NoRedundantContinueConfig>,
    pub no_literal_password: RuleSettings<NoLiteralPasswordConfig>,
    pub no_insecure_comparison: RuleSettings<NoInsecureComparisonConfig>,
}
