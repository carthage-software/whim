use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::config::FormatPatterns;
use crate::error::Error;

pub(super) struct Target {
    pub(super) path: PathBuf,
    pub(super) spelling: PathBuf,
}

pub(super) fn discover(
    paths: &[PathBuf],
    project_root: &Path,
    patterns: &FormatPatterns,
) -> Result<Vec<Target>, Error> {
    let mut files = Vec::new();
    if paths.is_empty() {
        collect_directory(
            project_root,
            Path::new(""),
            project_root,
            patterns,
            true,
            &mut files,
        )?;
    } else {
        for path in paths {
            collect_explicit(path, project_root, patterns, &mut files)?;
        }
    }

    let mut seen = HashSet::new();
    files.retain(|target| seen.insert(target.path.clone()));
    Ok(files)
}

fn collect_explicit(
    path: &Path,
    project_root: &Path,
    patterns: &FormatPatterns,
    files: &mut Vec<Target>,
) -> Result<(), Error> {
    let metadata = fs::metadata(path).map_err(|source| Error::InspectPath {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.is_file() {
        files.push(Target {
            path: resolve(path)?,
            spelling: path.to_path_buf(),
        });
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(Error::InvalidFormatTarget(path.to_path_buf()));
    }

    let resolved = resolve(path)?;
    let matching_root = if resolved.starts_with(project_root) {
        project_root
    } else {
        resolved.as_path()
    };
    if excluded(&resolved, matching_root, patterns) {
        return Ok(());
    }

    collect_directory(&resolved, path, matching_root, patterns, false, files)
}

fn collect_directory(
    directory: &Path,
    spelling: &Path,
    matching_root: &Path,
    patterns: &FormatPatterns,
    apply_include: bool,
    files: &mut Vec<Target>,
) -> Result<(), Error> {
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(directory)
        .map_err(|source| Error::ReadDirectory {
            path: directory.to_path_buf(),
            source,
        })?
        .collect::<Result<_, _>>()
        .map_err(|source| Error::ReadDirectory {
            path: directory.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(fs::DirEntry::file_name);

    for entry in entries {
        let path = entry.path();
        let relative =
            path.strip_prefix(matching_root)
                .map_err(|source| Error::FormatTargetEscapesRoot {
                    path: path.clone(),
                    root: matching_root.to_path_buf(),
                    source,
                })?;
        if patterns.excludes(relative) {
            continue;
        }

        let file_type = entry.file_type().map_err(|source| Error::InspectPath {
            path: path.clone(),
            source,
        })?;
        let child_spelling = spelling.join(entry.file_name());
        if file_type.is_dir() {
            collect_directory(
                &path,
                &child_spelling,
                matching_root,
                patterns,
                apply_include,
                files,
            )?;
        } else if file_type.is_file()
            && path.extension() == Some(OsStr::new("whim"))
            && (!apply_include || patterns.includes(relative))
        {
            files.push(Target {
                path,
                spelling: child_spelling,
            });
        }
    }

    Ok(())
}

fn excluded(path: &Path, root: &Path, patterns: &FormatPatterns) -> bool {
    path.strip_prefix(root)
        .is_ok_and(|relative| patterns.excludes(relative))
}

fn resolve(path: &Path) -> Result<PathBuf, Error> {
    fs::canonicalize(path).map_err(|source| Error::ResolvePath {
        path: path.to_path_buf(),
        source,
    })
}
