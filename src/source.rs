use std::fs;
use std::io;
use std::io::Read;
use std::path::PathBuf;

use crate::error::Error;

pub(crate) struct Source {
    pub(crate) file: PathBuf,
    pub(crate) text: String,
    pub(crate) path: PathBuf,
}

impl Source {
    pub(crate) fn read(file: PathBuf) -> Result<Self, Error> {
        let from_stdin = file.as_os_str() == "-";
        let text = if from_stdin {
            let mut source = String::new();
            io::stdin()
                .read_to_string(&mut source)
                .map_err(Error::ReadStdin)?;

            source
        } else {
            fs::read_to_string(&file).map_err(|source| Error::ReadFile {
                path: file.clone(),
                source,
            })?
        };

        let absolute = if from_stdin {
            PathBuf::from("-")
        } else {
            fs::canonicalize(&file).map_err(|source| Error::ResolvePath {
                path: file.clone(),
                source,
            })?
        };

        Ok(Self {
            file,
            text,
            path: absolute,
        })
    }
}
