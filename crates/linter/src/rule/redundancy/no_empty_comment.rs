use annotate_snippets::Level;
use indoc::indoc;
use whim_span::Span;
use whim_syn::arena::Arena;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::trivia::TriviaKind;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule::utils::comments::comment_lines;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

#[derive(Debug, Clone)]
pub struct NoEmptyCommentRule {
    meta: &'static RuleMeta,
    cfg: NoEmptyCommentConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct NoEmptyCommentConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
    #[cfg_attr(feature = "serde", serde(alias = "preserve-single-line-comments"))]
    pub preserve_single_line_comments: bool,
}

impl Default for NoEmptyCommentConfig {
    fn default() -> Self {
        Self {
            level: Level::NOTE,
            preserve_single_line_comments: false,
        }
    }
}

impl Config for NoEmptyCommentConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for NoEmptyCommentRule {
    type Config = NoEmptyCommentConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "No Empty Comment",
            code: "no-empty-comment",
            description: "Detects comments that contain no text.",
            good_example: indoc! {r"
                // Explain why this branch is safe.
            "},
            bad_example: "/**/",
            category: Category::Redundancy,
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

        let mut current_block = None;
        let mut pending = Vec::new();

        for trivia in program.trivia {
            if let Some((kind, end)) = &mut current_block {
                if (trivia.kind == *kind || trivia.kind == TriviaKind::Whitespace)
                    && trivia.span.start == *end
                {
                    *end = trivia.span.end;
                } else {
                    current_block = None;
                }
            }

            if current_block.is_none() {
                self.report_pending(ctx, &mut pending);
            }

            if !trivia.kind.is_comment()
                || (trivia.kind == TriviaKind::SingleLineComment
                    && self.cfg.preserve_single_line_comments)
            {
                continue;
            }

            let empty = comment_lines(trivia).all(|(_, line)| line.trim().is_empty());
            if empty {
                pending.push(trivia.span);
            } else if trivia.kind == TriviaKind::SingleLineComment {
                current_block = Some((trivia.kind, trivia.span.end));
                pending.clear();
            }
        }

        self.report_pending(ctx, &mut pending);
    }
}

impl NoEmptyCommentRule {
    fn report_pending<A: Arena>(&self, ctx: &mut LintContext<'_, '_, A>, pending: &mut Vec<Span>) {
        for span in pending.drain(..) {
            ctx.report(
                self.meta,
                self.cfg.level(),
                (span, "this comment contains no text"),
                "Empty comments are not allowed.",
                [],
                [Level::HELP
                    .message("Remove this comment or explain its purpose.")
                    .into()],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NoEmptyCommentRule;
    use crate::settings::Settings;
    use crate::test_lint_failure;
    use crate::test_lint_success;

    test_lint_failure! {
        name = empty_block_and_edge_line_comments_are_rejected,
        rule = NoEmptyCommentRule,
        count = 3,
        code = "//\n/**/\n//\n",
    }

    test_lint_success! {
        name = useful_comment_and_blank_line_inside_block_are_allowed,
        rule = NoEmptyCommentRule,
        code = "// First line.\n//\n// Last line.\n",
    }

    test_lint_success! {
        name = configured_single_line_comments_are_kept,
        rule = NoEmptyCommentRule,
        settings = |settings: &mut Settings| settings.rules.no_empty_comment.config.preserve_single_line_comments = true,
        code = "//\n",
    }
}
