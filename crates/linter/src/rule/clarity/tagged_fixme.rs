use annotate_snippets::Level;
use indoc::indoc;
use regex::Regex;

use std::sync::LazyLock;
use whim_span::Span;
use whim_syn::arena::Arena;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule::utils::comments::comment_lines;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

static TAGGED_FIXME_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"fixme\((#|@)?\S+").unwrap());

#[derive(Debug, Clone)]
pub struct TaggedFixmeRule {
    meta: &'static RuleMeta,
    cfg: TaggedFixmeConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct TaggedFixmeConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for TaggedFixmeConfig {
    fn default() -> Self {
        Self {
            level: Level::WARNING,
        }
    }
}

impl Config for TaggedFixmeConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for TaggedFixmeRule {
    type Config = TaggedFixmeConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "Tagged FIXME",
            code: "tagged-fixme",
            description: indoc! {"
                Detects FIXME comments that are not tagged with a user or issue reference. Untagged FIXME comments
                are not actionable and can be easily missed by the team. Tagging the FIXME comment with a user or
                issue reference ensures that the issue is tracked and resolved.
            "},
            good_example: indoc! {"
                // FIXME(@azjezz) This is a valid FIXME comment.
                // FIXME(azjezz) This is a valid FIXME comment.
                // FIXME(#123) This is a valid FIXME comment.
            "},
            bad_example: indoc! {"
                // FIXME: This is an invalid FIXME comment.
            "},
            category: Category::Clarity,
        };

        &META
    }

    fn targets() -> &'static [NodeKind] {
        &[NodeKind::Program]
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
        let Node::Program(program) = node else {
            return;
        };

        for trivia in program
            .trivia
            .iter()
            .filter(|trivia| trivia.kind.is_comment())
        {
            for (span, line) in comment_lines(trivia) {
                let text = line.trim_start().to_ascii_lowercase();
                if !text.starts_with("fixme") || TAGGED_FIXME_REGEX.is_match(&text) {
                    continue;
                }

                ctx.report(
                    self.meta,
                    self.cfg.level(),
                    (Span::new(span.start, span.start + 5), "missing an owner or issue reference"),
                    "Untagged FIXME comment.",
                    [],
                    [
                        Level::NOTE.message("A tag links this bug to someone responsible for it or to a tracked issue.").into(),
                        Level::HELP.message("Add a tag such as `FIXME(@name)`, `FIXME(name)`, or `FIXME(#123)`.").into(),
                    ],
                );

                break;
            }
        }
    }
}
