//! Limits shared by compilation, linking, optimization, and execution.

/// The greatest type recursion depth accepted by the runtime.
pub(crate) const MAX_TYPE_DEPTH: usize = 64;

/// The same limit for sites that use 32-bit depth counters.
pub(crate) const MAX_TYPE_DEPTH_U32: u32 = MAX_TYPE_DEPTH as u32;
