use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

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
            },
        )
        .unwrap_or_else(|error| panic!("failed to compile the standard library:\n{error}"));
    let bytes = artifact.into_bytes();

    let mut validator = Engine::new(EngineConfiguration::default());
    validator
        .load_artifact(&bytes)
        .unwrap_or_else(|error| panic!("failed to validate the standard library:\n{error}"));

    let output = output_directory.join("lib.whia");
    fs::write(&output, bytes).unwrap_or_else(|error| {
        panic!(
            "failed to write standard-library artifact {}: {error}",
            output.display()
        )
    });
}
