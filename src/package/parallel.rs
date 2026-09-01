use std::num::NonZeroUsize;
use std::thread;

use rayon::ThreadPoolBuildError;
use rayon::prelude::*;

const MAXIMUM_WORKERS: usize = 4;

pub(super) fn workers(jobs: usize) -> usize {
    if jobs == 0 {
        return 0;
    }

    thread::available_parallelism()
        .map_or(1, NonZeroUsize::get)
        .min(MAXIMUM_WORKERS)
        .min(jobs)
}

pub(super) fn try_map<T, U, E>(
    items: &[T],
    operation: impl Fn(&T) -> Result<U, E> + Send + Sync,
    pool_error: impl FnOnce(ThreadPoolBuildError) -> E,
) -> Result<Vec<U>, E>
where
    T: Sync,
    U: Send,
    E: Send,
{
    if items.len() < 2 {
        return items.iter().map(operation).collect();
    }

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers(items.len()))
        .thread_name(|index| format!("whim-package-{index}"))
        .build()
        .map_err(pool_error)?;

    let results = pool.install(|| items.par_iter().map(operation).collect::<Vec<_>>());
    results.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::try_map;
    use super::workers;

    #[test]
    fn worker_count_is_bounded_by_work_and_limit() {
        assert_eq!(0, workers(0));
        assert_eq!(1, workers(1));
        assert!((1..=4).contains(&workers(usize::MAX)));
    }

    #[test]
    fn parallel_map_preserves_input_order() {
        let values = [3_u8, 1, 2];
        let mapped = try_map(
            &values,
            |value| Ok::<_, String>(value * 2),
            |error| error.to_string(),
        )
        .expect("worker pool should start");

        assert_eq!(vec![6, 2, 4], mapped);
    }

    #[test]
    fn parallel_map_reports_errors_in_input_order() {
        let values = [2_u8, 1];
        let error = try_map(
            &values,
            |value| Err::<(), _>(value.to_string()),
            |error| error.to_string(),
        )
        .expect_err("the first operation should fail");

        assert_eq!("2", error);
    }
}
