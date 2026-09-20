//! Shared limits for Whim.

/// The greatest supported type recursion depth.
pub const MAX_TYPE_DEPTH: usize = 64;

/// The same limit for sites that use 32-bit depth counters.
pub const MAX_TYPE_DEPTH_U32: u32 = MAX_TYPE_DEPTH as u32;
