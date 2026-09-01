#[cfg(not(any(
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_env = "gnu", target_arch = "x86_64"),
    all(target_os = "linux", target_env = "gnu", target_arch = "aarch64"),
    all(target_os = "linux", target_env = "gnu", target_arch = "riscv64"),
)))]
compile_error!(
    "unsupported target: Whim only builds for \
     x86_64-apple-darwin, aarch64-apple-darwin, x86_64-unknown-linux-gnu, \
     aarch64-unknown-linux-gnu, riscv64gc-unknown-linux-gnu"
);
