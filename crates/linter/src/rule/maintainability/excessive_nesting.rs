use annotate_snippets::Level;
use indoc::indoc;

use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::cst::node::Node;
use whim_syn::cst::node::NodeKind;
use whim_syn::cst::walker::Flow;
use whim_syn::cst::walker::Visitor;
use whim_syn::cst::walker::walk;

use crate::category::Category;
use crate::context::LintContext;
use crate::rule::Config;
use crate::rule::LintRule;
use crate::rule_meta::RuleMeta;
use crate::settings::RuleSettings;

const DEFAULT_THRESHOLD: usize = 7;

#[derive(Debug, Clone)]
pub struct ExcessiveNestingRule {
    meta: &'static RuleMeta,
    cfg: ExcessiveNestingConfig,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default, rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct ExcessiveNestingConfig {
    #[cfg_attr(feature = "serde", serde(with = "crate::settings::level"))]
    pub level: Level<'static>,
    pub threshold: usize,
    pub function_like_threshold: Option<usize>,
}

impl Default for ExcessiveNestingConfig {
    fn default() -> Self {
        Self {
            level: Level::WARNING,
            threshold: DEFAULT_THRESHOLD,
            function_like_threshold: None,
        }
    }
}

impl Config for ExcessiveNestingConfig {
    fn level(&self) -> Level<'static> {
        self.level.clone()
    }
}

impl LintRule for ExcessiveNestingRule {
    type Config = ExcessiveNestingConfig;

    fn meta() -> &'static RuleMeta {
        const META: RuleMeta = RuleMeta {
            name: "Excessive Nesting",
            code: "excessive-nesting",
            description: indoc! {r"
                Checks if the nesting level in any block exceeds a configurable threshold.

                Deeply nested code is harder to read, understand, and maintain.
                Consider refactoring to use early returns, helper methods, or clearer control flow.

                The `function-like-threshold` option allows setting a separate, typically lower,
                threshold for individual functions, methods and closures.
            "},
            good_example: indoc! {r#"
                if ($condition) {
                    while ($otherCondition) {
                        write_line!("Hello"); // nesting depth = 2
                    }
                }
            "#},
            bad_example: indoc! {r#"
                if ($a) {
                    if ($b) {
                        if ($c) {
                            if ($d) {
                                if ($e) {
                                    if ($f) {
                                        if ($g) {
                                            if ($h) {
                                                write_line!("Too deeply nested!");
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            "#},
            category: Category::Maintainability,
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

        let mut walker = NestingWalker {
            ctx,
            meta: self.meta,
            cfg: &self.cfg,
            level: 0,
            threshold: self.cfg.threshold,
            function_like: false,
            scopes: Vec::new(),
        };

        walk(Node::Program(program), &mut walker);
    }
}

struct NestingWalker<'visit, 'ctx, 'arena, A: Arena> {
    ctx: &'visit mut LintContext<'ctx, 'arena, A>,
    meta: &'static RuleMeta,
    cfg: &'visit ExcessiveNestingConfig,
    level: usize,
    threshold: usize,
    function_like: bool,
    scopes: Vec<(usize, usize, bool)>,
}

impl<A: Arena> Visitor<'_, '_> for NestingWalker<'_, '_, '_, A> {
    fn enter(&mut self, node: Node<'_, '_>) -> Flow {
        match node {
            Node::Function(_) | Node::Method(_) | Node::Closure(_) => {
                self.scopes
                    .push((self.level, self.threshold, self.function_like));
                if let Some(threshold) = self.cfg.function_like_threshold {
                    self.level = 0;
                    self.threshold = threshold;
                    self.function_like = true;
                }
            }
            Node::Block(block) => {
                self.level += 1;
                if self.level > self.threshold {
                    let scope = if self.function_like {
                        "function"
                    } else {
                        "global"
                    };

                    self.ctx.report(
                        self.meta,
                        self.cfg.level(),
                        block.span(),
                        "Excessive block nesting.",
                        [
                            Level::NOTE
                                .message(format!(
                                    "This block has depth {}, above the {scope} threshold of {}.",
                                    self.level, self.threshold
                                ))
                                .into(),
                            Level::HELP
                                .message(
                                    "Use early returns or smaller functions to reduce nesting.",
                                )
                                .into(),
                        ],
                    );

                    return Flow::Skip;
                }
            }
            _ => {}
        }

        Flow::Descend
    }

    fn leave(&mut self, node: Node<'_, '_>) {
        match node {
            Node::Function(_) | Node::Method(_) | Node::Closure(_) => {
                if let Some((level, threshold, function_like)) = self.scopes.pop() {
                    self.level = level;
                    self.threshold = threshold;
                    self.function_like = function_like;
                }
            }
            Node::Block(_) => self.level -= 1,
            _ => {}
        }
    }
}
