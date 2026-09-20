use std::borrow::Cow;
use std::env::consts;

/// The values of platform constructs when compiling an artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub arch: Cow<'static, str>,
    pub os: Cow<'static, str>,
    pub family: Cow<'static, str>,
    pub dll_prefix: Cow<'static, str>,
    pub dll_suffix: Cow<'static, str>,
    pub dll_extension: Cow<'static, str>,
    pub exe_suffix: Cow<'static, str>,
    pub exe_extension: Cow<'static, str>,
}

impl Target {
    /// The platform running the compiler.
    pub const NATIVE: Self = Self {
        arch: Cow::Borrowed(consts::ARCH),
        os: Cow::Borrowed(consts::OS),
        family: Cow::Borrowed(consts::FAMILY),
        dll_prefix: Cow::Borrowed(consts::DLL_PREFIX),
        dll_suffix: Cow::Borrowed(consts::DLL_SUFFIX),
        dll_extension: Cow::Borrowed(consts::DLL_EXTENSION),
        exe_suffix: Cow::Borrowed(consts::EXE_SUFFIX),
        exe_extension: Cow::Borrowed(consts::EXE_EXTENSION),
    };
}
