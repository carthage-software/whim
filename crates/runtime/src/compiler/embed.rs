//! Compile-time file embedding.

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use hashbrown::HashMap;
use whim_span::Span;
use whim_syn::cst::atom::LiteralString;

use crate::compiler::error::CompileError;
use crate::compiler::error::CompileErrorKind;
use crate::path::path_from_bytes;
use crate::value::atom::Atom;
use crate::value::heap::Heap;

#[derive(Default)]
pub(crate) struct EmbeddedFiles {
    files: RefCell<HashMap<PathBuf, Atom>>,
}

impl EmbeddedFiles {
    pub(in crate::compiler) fn load(
        &self,
        heap: &Heap,
        source_path: &[u8],
        path: &LiteralString<'_>,
    ) -> Result<Atom, CompileError> {
        if source_path == b"-" {
            return Err(CompileError::new(
                CompileErrorKind::EmbeddedFileRequiresPath,
                "`embed!` cannot resolve a path in source read from standard input",
                path.span,
            ));
        }

        let relative = path_from_bytes(path.value);
        if relative.is_absolute() {
            return Err(CompileError::new(
                CompileErrorKind::AbsoluteEmbeddedFilePath,
                "`embed!` accepts only a relative path",
                path.span,
            ));
        }

        let source = path_from_bytes(source_path);
        let resolved = source
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(relative);
        if let Some(contents) = self.files.borrow().get(&resolved) {
            return Ok(contents.clone());
        }

        let canonical = fs::canonicalize(&resolved)
            .map_err(|error| embedded_file_error(&resolved, &error, path.span))?;
        let cached = self.files.borrow().get(&canonical).cloned();
        if let Some(contents) = cached {
            self.files.borrow_mut().insert(resolved, contents.clone());
            return Ok(contents);
        }

        let contents = fs::read(&canonical)
            .map_err(|error| embedded_file_error(&canonical, &error, path.span))?;
        let contents = heap.intern(&contents);
        let mut files = self.files.borrow_mut();
        files.insert(canonical, contents.clone());
        files.insert(resolved, contents.clone());
        Ok(contents)
    }
}

fn embedded_file_error(path: &Path, error: &io::Error, span: Span) -> CompileError {
    CompileError::new(
        CompileErrorKind::ReadEmbeddedFile,
        format!("could not embed `{}`: {error}", path.display()),
        span,
    )
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::io::ErrorKind;
    use std::process;

    use whim_span::Span;
    use whim_syn::cst::atom::LiteralString;
    use whim_syn::cst::atom::LiteralStringKind;

    use super::EmbeddedFiles;
    use crate::path::path_bytes;
    use crate::value::heap::Heap;

    #[test]
    fn one_compilation_reads_each_embedded_file_once() {
        let directory = env::temp_dir().join(format!("whim-embed-cache-test-{}", process::id()));
        match fs::remove_dir_all(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => panic!("the old test directory could not be removed: {error}"),
        }
        fs::create_dir(&directory).expect("the test directory is creatable");

        let asset = directory.join("asset.bin");
        fs::write(&asset, b"first").expect("the asset is writable");
        let source = directory.join("source.whim");
        let path = LiteralString {
            kind: LiteralStringKind::SingleQuoted,
            span: Span::zero(),
            raw: "'asset.bin'",
            value: b"asset.bin",
        };
        let heap = Heap::new();
        let embedded = EmbeddedFiles::default();

        let first = embedded
            .load(&heap, &path_bytes(&source), &path)
            .expect("the asset is readable");
        fs::remove_file(&asset).expect("the asset can be removed");
        let second = embedded
            .load(&heap, &path_bytes(&source), &path)
            .expect("the cached asset remains readable");

        assert_eq!(first.as_bytes(), b"first");
        assert_eq!(second.as_bytes(), b"first");
        fs::remove_dir_all(&directory).expect("the test directory is removable");
    }
}
