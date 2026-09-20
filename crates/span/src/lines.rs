#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "source offsets and line counts are limited to u32"
)]
pub fn line_of(line_starts: &[u32], offset: u32) -> u32 {
    if line_starts.is_empty() {
        return 0;
    }

    line_starts.partition_point(|start| *start <= offset) as u32
}

/// Returns a vec over the starting byte offsets of each line in `source`.
///
/// Stolen from [Mago](https://github.com/carthage-software/mago/blob/116abff82d8cd9556dc3be16d5de8e199c7108d2/crates/database/src/file.rs#L284).
#[inline]
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "source offsets are limited to u32"
)]
pub fn line_starts_of(source: &str) -> Vec<u32> {
    // Heuristic: On the test corpus, the mean length is about 30 bytes, the median is 23.
    // Since the whole vec will be small, we prefer slight over-allocation to avoid re-allocations
    // in the common case
    const LINE_WIDTH_HEURISTIC: usize = 20;

    let source = source.as_bytes();

    // Pre-allocate to avoid calling `realloc` thousands of times per file.
    let mut lines = Vec::with_capacity(source.len() / LINE_WIDTH_HEURISTIC);
    lines.push(0);

    // Detect line ending style from the first \r or \n.  Real files use one
    // convention throughout, so we never need to handle mixed \r\n / bare \r.
    match memchr::memchr2(b'\r', b'\n', source) {
        // No line endings: single-line file, nothing more to push.
        None => {}
        // Old Mac (\r only): first line-ending char is a bare \r.
        Some(cr) if source[cr] == b'\r' && source.get(cr + 1) != Some(&b'\n') => {
            for pos in memchr::memchr_iter(b'\r', source) {
                lines.push((pos + 1) as u32);
            }
        }
        // Unix (\n only) or Windows (\r\n): \n marks every line start.
        _ => {
            for pos in memchr::memchr_iter(b'\n', source) {
                lines.push((pos + 1) as u32);
            }
        }
    }

    lines
}
