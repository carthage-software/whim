use std::sync::LazyLock;

use annotate_snippets::Level;
use indoc::indoc;
use regex::Regex;

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

static TAGGED_TODO_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"todo\((#|@)?\S+").unwrap());

#[derive(Debug, Clone)]
pub struct TaggedTodoRule {
    meta: &'static RuleMeta,
    cfg: TaggedTodoConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct TaggedTodoConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
}

impl Default for TaggedTodoConfig {
    fn default() -> Self {
        Self {
            level: Level::WARNING,
        }
    }
}

impl Config for TaggedTodoConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for TaggedTodoRule {
    type Config = TaggedTodoConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "Tagged TODO",
            code: "tagged-todo",
            description: indoc! {"
                Detects TODO comments that are not tagged with a user or issue reference. Untagged TODOs
                can be difficult to track and may be forgotten. Tagging TODOs with a user or issue reference
                makes it easier to track progress and ensures that tasks are not forgotten.
            "},
            good_example: indoc! {r"
                // TODO(@azjezz) This is a valid TODO comment.
                // TODO(azjezz) This is a valid TODO comment.
                // TODO(#123) This is a valid TODO comment.
            "},
            bad_example: indoc! {r"
                // TODO: This is an invalid TODO comment.
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
            for line in comment_lines(trivia) {
                let text = line.trim_start().to_ascii_lowercase();
                if !text.starts_with("todo") || TAGGED_TODO_REGEX.is_match(&text) {
                    continue;
                }

                ctx.report(
                    self.meta,
                    self.cfg.level(),
                    trivia.span,
                    "TODO should be tagged with (@username) or (#issue).",
                    [Level::HELP
                        .message(
                            "Add a user tag or issue reference, such as TODO(@name) or TODO(#123).",
                        )
                        .into()],
                );

                break;
            }
        }
    }
}
