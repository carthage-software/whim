use annotate_snippets::Level;
use indoc::indoc;
use std::str::from_utf8;
use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

const DEFAULT_MIN_DIGITS: usize = 5;

#[derive(Debug, Clone)]
pub struct ReadableLiteralRule {
    meta: &'static RuleMeta,
    cfg: ReadableLiteralConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct ReadableLiteralConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
    #[cfg_attr(feature = "serde", serde(alias = "min-digits"))]
    pub min_digits: usize,
}

impl Default for ReadableLiteralConfig {
    fn default() -> Self {
        Self {
            level: Level::WARNING,
            min_digits: DEFAULT_MIN_DIGITS,
        }
    }
}

impl Config for ReadableLiteralConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for ReadableLiteralRule {
    type Config = ReadableLiteralConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "Readable Literal",
            code: "readable-literal",
            description: "Suggests underscore separators in long numeric literals.",
            good_example: indoc! {r"
                $amount = 1_000_000;
                $mask = 0xCAFE_F00D;
            "},
            bad_example: indoc! {r"
                $amount = 1000000;
                $mask = 0xCAFEF00D;
            "},
            category: Category::Clarity,
        };
        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::LiteralInteger, NodeKind::LiteralFloat]
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
        let (raw, span) = match node {
            Node::LiteralInteger(literal) => (literal.raw, literal.span()),
            Node::LiteralFloat(literal) => (literal.raw, literal.span()),
            _ => return,
        };
        if raw.contains('_') || count_significant_digits(raw) < self.cfg.min_digits {
            return;
        }

        let suggestion = suggest_separated_literal(raw);
        ctx.report(
            self.meta,
            self.cfg.level(),
            (span, "this number is hard to scan"),
            "Numeric literal could use underscore separators.",
            [],
            [
                Level::NOTE
                    .message("Separators do not change the numeric value.")
                    .into(),
                Level::HELP
                    .message(format!("Write this as `{suggestion}`."))
                    .into(),
            ],
        );
    }
}

fn count_significant_digits(raw: &str) -> usize {
    if let Some(digits) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        return digits.chars().filter(char::is_ascii_hexdigit).count();
    }
    if let Some(digits) = raw.strip_prefix("0b").or_else(|| raw.strip_prefix("0B")) {
        return digits
            .chars()
            .filter(|digit| matches!(digit, '0' | '1'))
            .count();
    }
    if let Some(digits) = raw.strip_prefix("0o").or_else(|| raw.strip_prefix("0O")) {
        return digits
            .chars()
            .filter(|digit| matches!(digit, '0'..='7'))
            .count();
    }

    let mantissa = raw
        .find(['e', 'E'])
        .map_or(raw, |position| &raw[..position]);
    if let Some(position) = mantissa.find('.') {
        let left = mantissa[..position]
            .chars()
            .filter(char::is_ascii_digit)
            .count();
        let right = mantissa[position + 1..]
            .chars()
            .filter(char::is_ascii_digit)
            .count();
        return left.max(right);
    }
    mantissa.chars().filter(char::is_ascii_digit).count()
}

fn suggest_separated_literal(raw: &str) -> String {
    for (lower, upper, size) in [("0x", "0X", 4), ("0b", "0B", 4), ("0o", "0O", 3)] {
        if let Some(digits) = raw.strip_prefix(lower).or_else(|| raw.strip_prefix(upper)) {
            return format!("{}{}", &raw[..2], group_from_right(digits, size));
        }
    }

    let exponent = raw.find(['e', 'E']);
    let (mantissa, exponent) =
        exponent.map_or((raw, ""), |position| (&raw[..position], &raw[position..]));
    let (integer, fraction) = mantissa.find('.').map_or((mantissa, None), |position| {
        (&mantissa[..position], Some(&mantissa[position + 1..]))
    });
    let mut result = if integer.len() >= 4 {
        group_from_right(integer, 3)
    } else {
        integer.to_owned()
    };
    if let Some(fraction) = fraction {
        result.push('.');
        if fraction.len() >= 4 {
            result.push_str(&group_from_left(fraction, 3));
        } else {
            result.push_str(fraction);
        }
    }
    result.push_str(exponent);
    result
}

fn group_from_right(digits: &str, size: usize) -> String {
    let first = digits.len() % size;
    let first = if first == 0 { size } else { first };
    let mut result = String::with_capacity(digits.len() + digits.len() / size);
    result.push_str(&digits[..first]);
    for chunk in digits.as_bytes()[first..].chunks(size) {
        result.push('_');
        result.push_str(from_utf8(chunk).unwrap_or_default());
    }
    result
}

fn group_from_left(digits: &str, size: usize) -> String {
    let mut result = String::with_capacity(digits.len() + digits.len() / size);
    for (index, chunk) in digits.as_bytes().chunks(size).enumerate() {
        if index != 0 {
            result.push('_');
        }
        result.push_str(from_utf8(chunk).unwrap_or_default());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::ReadableLiteralRule;
    use crate::settings::Settings;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = long_literals_are_rejected,
        rule = ReadableLiteralRule,
        count = 4,
        code = "$a = 1000000; $b = 0xCAFEF00D; $c = 0b01011111; $d = 12345.67890;",
    }

    test_lint_success! {
        name = short_and_separated_literals_are_allowed,
        rule = ReadableLiteralRule,
        code = "$a = 1234; $b = 1_000_000; $c = 0o7_777;",
    }

    test_lint_failure! {
        name = minimum_digit_setting_is_used,
        rule = ReadableLiteralRule,
        settings = |settings: &mut Settings| settings.rules.readable_literal.config.min_digits = 4,
        code = "$a = 1234;",
    }
}
