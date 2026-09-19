#[cfg(not(any(
    all(
        any(target_os = "macos", target_os = "freebsd"),
        any(target_arch = "x86_64", target_arch = "aarch64"),
    ),
    all(
        target_os = "linux",
        any(target_env = "gnu", target_env = "musl"),
        any(target_arch = "x86_64", target_arch = "aarch64"),
    ),
    all(target_os = "linux", target_env = "gnu", target_arch = "riscv64"),
)))]
compile_error!(
    "unsupported target: Whim only builds for \
     x86_64-apple-darwin, aarch64-apple-darwin, x86_64-unknown-linux-gnu, \
     aarch64-unknown-linux-gnu, riscv64gc-unknown-linux-gnu, \
     x86_64-unknown-linux-musl, aarch64-unknown-linux-musl, \
     x86_64-unknown-freebsd, aarch64-unknown-freebsd"
);
