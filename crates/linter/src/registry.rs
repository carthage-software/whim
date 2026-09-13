use std::path::Path;

use globset::GlobBuilder;
use globset::GlobSet;
use globset::GlobSetBuilder;

use whim_syn::cst::node::NodeKind;

use crate::rule::AnyRule;
use crate::settings::Settings;

#[derive(Debug, Clone)]
pub struct RuleRegistry {
    rules: Vec<AnyRule>,
    enabled_count: usize,
    rule_excludes: Vec<GlobSet>,
    by_kind: Vec<Box<[usize]>>,
}

impl RuleRegistry {
    pub fn build(
        settings: &Settings,
        only: Option<&[String]>,
        include_disabled: bool,
    ) -> Result<Self, globset::Error> {
        let mut rules = Vec::new();
        let mut rule_excludes = Vec::new();
        let (enabled, disabled): (Vec<_>, Vec<_>) = AnyRule::get_all_for(settings, only, true)
            .into_iter()
            .partition(|(rule, _)| {
                only.is_some() || include_disabled || rule.is_enabled_in(settings)
            });
        let enabled_count = enabled.len();
        for (rule, patterns) in enabled.into_iter().chain(disabled) {
            let mut excludes = GlobSetBuilder::new();
            for pattern in patterns {
                excludes.add(
                    GlobBuilder::new(&pattern)
                        .literal_separator(true)
                        .backslash_escape(false)
                        .build()?,
                );
            }
            rule_excludes.push(excludes.build()?);
            rules.push(rule);
        }
        let mut by_kind = vec![Vec::new(); usize::from(u8::MAX) + 1];
        for (index, rule) in rules.iter().enumerate() {
            for &kind in rule.targets() {
                by_kind[kind as usize].push(index);
            }
        }
        Ok(Self {
            rules,
            enabled_count,
            rule_excludes,
            by_kind: by_kind.into_iter().map(Vec::into_boxed_slice).collect(),
        })
    }

    #[must_use]
    pub fn rules(&self) -> &[AnyRule] {
        &self.rules[..self.enabled_count]
    }

    pub(crate) fn all_rules(&self) -> &[AnyRule] {
        &self.rules
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.enabled_count
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.enabled_count == 0
    }

    #[must_use]
    pub fn is_rule_enabled(&self, code: &str) -> bool {
        self.rules().iter().any(|rule| rule.code() == code)
    }

    #[must_use]
    pub fn for_kind(&self, kind: NodeKind) -> &[usize] {
        &self.by_kind[kind as usize]
    }

    #[must_use]
    pub fn rule(&self, index: usize) -> &AnyRule {
        &self.rules[index]
    }

    #[must_use]
    pub fn excludes(&self, index: usize, path: &Path) -> bool {
        self.rule_excludes[index].is_match(path)
    }
}
