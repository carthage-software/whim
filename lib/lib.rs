//! The official Whim standard-library artifact.

use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineError;

static ARTIFACT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/lib.whia"));

/// Loads the embedded standard library into `engine`.
///
/// # Errors
///
/// Returns an error when decoding, declaration, or top-level execution fails.
pub fn load(engine: &mut Engine) -> Result<(), EngineError> {
    // SAFETY: the build script verifies these exact bytes before embedding them.
    unsafe { engine.load_verified_static_artifact(ARTIFACT) }
}
