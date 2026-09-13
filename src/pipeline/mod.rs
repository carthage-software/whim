use std::ops::ControlFlow;
use std::time::Instant;

use rayon::prelude::*;
use tracing::Span;
use whim_syn::arena::LocalArena;

use crate::error::Error;
use crate::pipeline::files::Target;

pub(crate) mod files;

const REPORTING_BATCH: usize = 64;

pub(crate) trait Reducer<I> {
    type Output;

    fn reduce(&mut self, targets: &[Target], results: Vec<I>) -> Result<ControlFlow<()>, Error>;

    fn finish(self) -> Result<Self::Output, Error>;
}

pub(crate) struct StatelessParallelPipeline<'files, T> {
    targets: &'files [Target],
    context: T,
}

impl<'files, T: Sync> StatelessParallelPipeline<'files, T> {
    pub(crate) const fn new(targets: &'files [Target], context: T) -> Self {
        Self { targets, context }
    }

    #[tracing::instrument(
        name = "pipeline",
        level = "debug",
        skip_all,
        fields(files = self.targets.len(), batch_size = REPORTING_BATCH),
    )]
    pub(crate) fn run<I: Send, R: Reducer<I>>(
        &self,
        map: impl Fn(&T, &LocalArena, &Target) -> I + Sync,
        mut reducer: R,
    ) -> Result<R::Output, Error> {
        let start = tracing::enabled!(tracing::Level::DEBUG).then(Instant::now);
        if !self.targets.is_empty() {
            tracing::debug!(
                threads = rayon::current_num_threads(),
                "processing files in parallel"
            );
        }

        for (index, batch) in self.targets.chunks(REPORTING_BATCH).enumerate() {
            let batch_span = tracing::trace_span!("batch", index, files = batch.len());
            let _entered = batch_span.enter();
            let parent = Span::current();
            let results: Vec<_> = timed("map", || {
                batch
                    .par_iter()
                    .map_init(LocalArena::new, |arena, target| {
                        parent.in_scope(|| {
                            let file_span =
                                tracing::trace_span!("file", path = %target.spelling.display());
                            let _entered = file_span.enter();
                            let result = timed("process", || map(&self.context, arena, target));
                            arena.reset();
                            result
                        })
                    })
                    .collect()
            });

            let flow = timed("reduce", || reducer.reduce(batch, results))?;
            if flow.is_break() {
                break;
            }
        }

        let result = timed("flush", || reducer.finish());
        if let Some(start) = start {
            tracing::debug!(elapsed = ?start.elapsed(), "finished file pipeline");
        }

        result
    }
}

pub(crate) fn timed<T>(phase: &'static str, operation: impl FnOnce() -> T) -> T {
    let start = tracing::enabled!(tracing::Level::TRACE).then(Instant::now);
    let result = operation();
    if let Some(start) = start {
        tracing::trace!(phase, elapsed = ?start.elapsed(), "finished phase");
    }

    result
}
