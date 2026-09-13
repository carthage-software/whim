use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use crate::config::Configuration;
use crate::config::FilePatterns;
use crate::error::Error;

pub(crate) struct Target {
    pub(crate) path: PathBuf,
    pub(crate) spelling: PathBuf,
}

pub(crate) struct Selection<'root> {
    pub(crate) root: &'root Path,
    pub(crate) targets: Vec<Target>,
}

impl<'root> Selection<'root> {
    pub(crate) fn new(
        paths: &[PathBuf],
        configuration: &'root Configuration,
        patterns: &FilePatterns,
    ) -> Result<Self, Error> {
        if let Ok(manifest) = configuration.manifest() {
            tracing::debug!(path = %manifest.path().display(), "selected configuration");
        } else {
            tracing::debug!("using default configuration");
        }

        let root = if paths.is_empty() {
            configuration.project_root()?
        } else {
            configuration.root()
        };

        Ok(Self {
            root,
            targets: discover(paths, root, patterns)?,
        })
    }
}

#[tracing::instrument(
    level = "debug",
    skip_all,
    fields(root = %project_root.display(), project = paths.is_empty(), paths = paths.len()),
    err(level = "debug"),
)]
pub(crate) fn discover(
    paths: &[PathBuf],
    project_root: &Path,
    patterns: &FilePatterns,
) -> Result<Vec<Target>, Error> {
    let start = Instant::now();
    let mut discovery = Discovery {
        patterns,
        files: Vec::new(),
        seen: HashSet::new(),
        directories: 0,
        excluded: 0,
        not_included: 0,
        duplicates: 0,
    };

    if paths.is_empty() {
        discovery.directory(project_root, project_root, project_root, true)?;
    } else {
        for path in paths {
            discovery.explicit(path, project_root)?;
        }
    }

    tracing::debug!(
        files = discovery.files.len(),
        directories = discovery.directories,
        excluded = discovery.excluded,
        not_included = discovery.not_included,
        duplicates = discovery.duplicates,
        elapsed = ?start.elapsed(),
        "finished file discovery",
    );

    if discovery.files.is_empty() {
        tracing::info!("no matching source files");
    }

    Ok(discovery.files)
}

struct Discovery<'patterns> {
    patterns: &'patterns FilePatterns,
    files: Vec<Target>,
    seen: HashSet<PathBuf>,
    directories: usize,
    excluded: usize,
    not_included: usize,
    duplicates: usize,
}

impl Discovery<'_> {
    fn explicit(&mut self, path: &Path, project_root: &Path) -> Result<(), Error> {
        tracing::debug!(path = %path.display(), "inspecting explicit path");
        let metadata = fs::metadata(path).map_err(|source| Error::InspectPath {
            path: path.to_path_buf(),
            source,
        })?;

        let resolved = fs::canonicalize(path).map_err(|source| Error::ResolvePath {
            path: path.to_path_buf(),
            source,
        })?;

        if metadata.is_file() {
            self.push(resolved, path.to_path_buf());
            return Ok(());
        }

        if !metadata.is_dir() {
            return Err(Error::InvalidFileTarget(path.to_path_buf()));
        }

        let matching_root = if resolved.starts_with(project_root) {
            project_root
        } else {
            resolved.as_path()
        };

        if self.excluded(&resolved, matching_root) {
            return Ok(());
        }

        self.directory(&resolved, path, matching_root, false)
    }

    fn directory(
        &mut self,
        directory: &Path,
        spelling: &Path,
        matching_root: &Path,
        apply_include: bool,
    ) -> Result<(), Error> {
        tracing::trace!(path = %directory.display(), apply_include, "reading directory");
        self.directories += 1;
        let mut entries: Vec<fs::DirEntry> = fs::read_dir(directory)
            .and_then(Iterator::collect)
            .map_err(|source| Error::ReadDirectory {
                path: directory.to_path_buf(),
                source,
            })?;

        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let relative = path.strip_prefix(matching_root).map_err(|source| {
                Error::FileTargetEscapesRoot {
                    path: path.clone(),
                    root: matching_root.to_path_buf(),
                    source,
                }
            })?;

            if self.excluded(&path, matching_root) {
                continue;
            }

            let file_type = entry.file_type().map_err(|source| Error::InspectPath {
                path: path.clone(),
                source,
            })?;

            let child_spelling = spelling.join(entry.file_name());
            if file_type.is_dir() {
                self.directory(&path, &child_spelling, matching_root, apply_include)?;
            } else if !file_type.is_file() {
                tracing::trace!(path = %path.display(), reason = "not a regular file", "skipped path");
            } else if path.extension() != Some(OsStr::new("whim")) {
                tracing::trace!(path = %path.display(), reason = "extension", "skipped path");
            } else if apply_include && !self.patterns.includes(relative) {
                self.not_included += 1;
                tracing::trace!(path = %path.display(), reason = "include filter", "skipped path");
            } else {
                self.push(path, child_spelling);
            }
        }

        Ok(())
    }

    fn excluded(&mut self, path: &Path, root: &Path) -> bool {
        let excluded = path
            .strip_prefix(root)
            .is_ok_and(|relative| self.patterns.excludes(relative));

        if excluded {
            self.excluded += 1;
            tracing::trace!(path = %path.display(), reason = "exclude filter", "skipped path");
        }

        excluded
    }

    fn push(&mut self, path: PathBuf, spelling: PathBuf) {
        if self.seen.insert(path.clone()) {
            tracing::trace!(file = %spelling.display(), resolved = %path.display(), "selected file");
            self.files.push(Target { path, spelling });
        } else {
            self.duplicates += 1;
            tracing::trace!(path = %spelling.display(), reason = "duplicate", "skipped path");
        }
    }
}
