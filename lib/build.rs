use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use whim_compiler::target::Target;
use whim_runtime::artifact::ArtifactConfiguration;
use whim_runtime::artifact::SourceFile;
use whim_runtime::engine::Engine;
use whim_runtime::engine::EngineConfiguration;

fn whim_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .unwrap_or_else(|error| {
            panic!(
                "failed to read standard-library directory {}: {error}",
                directory.display()
            )
        })
        .map(|entry| {
            entry.unwrap_or_else(|error| {
                panic!("failed to read a standard-library directory entry: {error}")
            })
        })
        .collect();

    entries.sort_by_key(fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type().unwrap_or_else(|error| {
            panic!(
                "failed to inspect standard-library path {}: {error}",
                path.display()
            )
        });

        if file_type.is_symlink() {
            panic!(
                "standard-library source tree contains a symbolic link: {}",
                path.display()
            );
        }

        if file_type.is_dir() {
            whim_files(&path, files);
        } else if file_type.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "whim")
        {
            files.push(path);
        }
    }
}

fn artifact_path(root: &Path, path: &Path) -> String {
    let relative = path
        .strip_prefix(root)
        .unwrap_or_else(|_| panic!("{} is outside {}", path.display(), root.display()));
    let mut result = String::from("<std>");
    for component in relative.components() {
        result.push('/');
        result.push_str(component.as_os_str().to_str().unwrap_or_else(|| {
            panic!(
                "standard-library source path is not UTF-8: {}",
                path.display()
            )
        }));
    }
    result
}

fn main() {
    let target =
        env::var("TARGET").unwrap_or_else(|_| panic!("the Cargo build did not provide `TARGET`"));
    let host =
        env::var("HOST").unwrap_or_else(|_| panic!("the Cargo build did not provide `HOST`"));
    let cross_compile = target != host;
    let manifest = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR")
            .unwrap_or_else(|| panic!("the Cargo build did not provide `CARGO_MANIFEST_DIR`")),
    );
    let source_root = manifest.join("src");
    let output_directory = PathBuf::from(
        env::var_os("OUT_DIR")
            .unwrap_or_else(|| panic!("the Cargo build did not provide `OUT_DIR`")),
    );
    println!("cargo:rerun-if-changed={}", source_root.display());

    let mut files = Vec::new();
    whim_files(&source_root, &mut files);
    let sources: Vec<_> = files
        .iter()
        .map(|path| {
            let contents = fs::read_to_string(path).unwrap_or_else(|error| {
                panic!(
                    "failed to read standard-library source {}: {error}",
                    path.display()
                )
            });
            (artifact_path(&source_root, path), contents)
        })
        .collect();
    let source_files: Vec<_> = sources
        .iter()
        .map(|(path, contents)| SourceFile::new(path, contents))
        .collect();

    let mut engine = Engine::new(EngineConfiguration::default());
    let artifact = engine
        .compile_artifact(
            "<std>/lib.whim",
            &source_files,
            ArtifactConfiguration {
                optimize: true,
                trusted_return_types: true,
                target: cross_compile.then(|| compilation_target(&target)),
            },
        )
        .unwrap_or_else(|error| panic!("failed to compile the standard library:\n{error}"));
    let bytes = artifact.into_bytes();

    if !cross_compile {
        let mut validator = Engine::new(EngineConfiguration::default());
        validator
            .load_artifact(&bytes)
            .unwrap_or_else(|error| panic!("failed to validate the standard library:\n{error}"));
    }

    let output = output_directory.join("lib.whia");
    fs::write(&output, bytes).unwrap_or_else(|error| {
        panic!(
            "failed to write standard-library artifact {}: {error}",
            output.display()
        )
    });
}

fn compilation_target(triple: &str) -> Target {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").expect("Cargo provides the target architecture");
    let os = env::var("CARGO_CFG_TARGET_OS").expect("Cargo provides the target operating system");
    let family = env::var("CARGO_CFG_TARGET_FAMILY").unwrap_or_default();
    let executable = target_file_name(triple, "bin");
    let library = target_file_name(triple, "cdylib");
    let exe_suffix = executable.strip_prefix("whim_target").unwrap_or("");
    let (dll_prefix, dll_suffix) = library.split_once("whim_target").unwrap_or(("", ""));
    Target {
        arch: arch.into(),
        os: os.into(),
        family: family.split(',').next().unwrap_or("").to_string().into(),
        dll_prefix: dll_prefix.to_string().into(),
        dll_suffix: dll_suffix.to_string().into(),
        dll_extension: dll_suffix.trim_start_matches('.').to_string().into(),
        exe_suffix: exe_suffix.to_string().into(),
        exe_extension: exe_suffix.trim_start_matches('.').to_string().into(),
    }
}

fn target_file_name(triple: &str, crate_type: &str) -> String {
    let output = Command::new(env::var_os("RUSTC").expect("Cargo provides the Rust compiler"))
        .args([
            "--print",
            "file-names",
            "--crate-name",
            "whim_target",
            "--crate-type",
            crate_type,
            "-C",
            "target-feature=-crt-static",
            "--target",
            triple,
            "-",
        ])
        .stdin(Stdio::null())
        .output()
        .expect("the Rust compiler reports target file names");
    assert!(
        output.status.success(),
        "failed to query target {triple}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("Rust target file names are UTF-8")
        .trim()
        .to_string()
}
