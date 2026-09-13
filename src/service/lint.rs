use std::path::Path;
use std::sync::Arc;

use annotate_snippets::Level;
use annotate_snippets::Renderer;

use whim_linter::Linter;
use whim_linter::registry::RuleRegistry;
use whim_span::Span;
use whim_syn::arena::LocalArena;
use whim_syn::parser::parse;

use crate::error::Error;
use crate::pipeline::StatelessParallelPipeline;
use crate::pipeline::files::Target;
use crate::pipeline::timed;
use crate::service::FileResult;
use crate::service::FileStatus;
use crate::service::RunSummary;
use crate::service::output::DiagnosticCounts;
use crate::service::output::OutputReducer;
use crate::service::read;
use crate::service::syntax;

pub(crate) struct LintService<'root> {
    registry: Arc<RuleRegistry>,
    minimum_fail_level: Level<'static>,
    root: &'root Path,
    color: bool,
}

impl<'root> LintService<'root> {
    pub(crate) fn new(
        registry: RuleRegistry,
        minimum_fail_level: Level<'static>,
        root: &'root Path,
        color: bool,
    ) -> Self {
        Self {
            registry: Arc::new(registry),
            minimum_fail_level,
            root,
            color,
        }
    }

    pub(crate) fn run(&self, targets: &[Target]) -> Result<RunSummary, Error> {
        StatelessParallelPipeline::new(targets, self).run(
            |service, arena, target| service.process(arena, target),
            OutputReducer::new()?,
        )
    }

    fn process(&self, arena: &LocalArena, target: &Target) -> FileResult {
        let source = read(target)?;
        let program = timed("parse", || parse(arena, &source))
            .map_err(|error| syntax(error, &source, target, self.color))?;
        let linter = Linter::from_registry(arena, Arc::clone(&self.registry));
        let mut counts = DiagnosticCounts::default();
        let mut failed = false;
        let mut on_diagnostic = |_: Span, _: &str, level: &Level<'static>, _: &str| {
            counts.record(level);
            failed |= self.minimum_fail_level == Level::NOTE
                || (*level != Level::NOTE && *level <= self.minimum_fail_level);
        };

        let name = target.spelling.to_string_lossy();
        let matching_path = target.path.strip_prefix(self.root).unwrap_or(&target.path);
        let groups = timed("lint", || {
            linter.lint_with_diagnostics(&name, matching_path, program, Some(&mut on_diagnostic))
        });
        if groups.is_empty() {
            return Ok(FileStatus::Unchanged);
        }
        let renderer = if self.color {
            Renderer::styled()
        } else {
            Renderer::plain()
        };
        let text = timed("render", || format!("{}\n", renderer.render(&groups)));
        Ok(FileStatus::Diagnostics {
            text,
            counts,
            failed,
        })
    }
}
