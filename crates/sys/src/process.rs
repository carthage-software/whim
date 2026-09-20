use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::path::path_bytes;
use crate::{Descriptor, Error, Result};

pub use crate::platform::process::*;

pub enum Stream {
    Inherit,
    Null,
    Pipe,
    File(File),
    Terminal,
}

impl Stream {
    fn stdio(self, input: bool) -> Result<Stdio> {
        match self {
            Self::Inherit => Ok(Stdio::inherit()),
            Self::Null => Ok(Stdio::null()),
            Self::Pipe => Ok(Stdio::piped()),
            Self::File(file) => Ok(file.into()),
            Self::Terminal => terminal_stream(input).map(Stdio::from),
        }
    }
}

pub struct Spawn {
    pub program: PathBuf,
    pub arguments: Vec<OsString>,
    pub environment: Option<Vec<(OsString, OsString)>>,
    pub directory: Option<PathBuf>,
    pub streams: [Stream; 3],
    pub inherited: Vec<(File, i32)>,
    pub group: Option<i32>,
}

impl Spawn {
    pub(crate) fn command(self) -> Result<Prepared> {
        validate_spawn(!self.inherited.is_empty(), self.group.is_some())?;
        let mut command = Command::new(self.program);
        command.args(self.arguments);
        if let Some(environment) = self.environment {
            for (name, _) in &environment {
                validate_environment_name(name)?;
            }
            command.env_clear().envs(environment);
        }
        if let Some(directory) = self.directory {
            command.current_dir(directory);
        }
        let [input, output, error] = self.streams;
        command
            .stdin(input.stdio(true)?)
            .stdout(output.stdio(false)?)
            .stderr(error.stdio(false)?);
        Ok(Prepared {
            command,
            #[cfg(unix)]
            inherited: self.inherited,
            #[cfg(unix)]
            group: self.group,
        })
    }
}

pub(crate) struct Prepared {
    pub command: Command,
    #[cfg(unix)]
    pub inherited: Vec<(File, i32)>,
    #[cfg(unix)]
    pub group: Option<i32>,
}

pub struct Spawned {
    pub id: u32,
    pub input: Option<Descriptor>,
    pub output: Option<Descriptor>,
    pub error: Option<Descriptor>,
}

pub struct Exit {
    pub status: i64,
    pub user_time: u64,
    pub system_time: u64,
}

pub(crate) fn validate_environment_name(name: &OsStr) -> Result<()> {
    let bytes = path_bytes(Path::new(name));
    #[cfg(unix)]
    let invalid = bytes.is_empty() || bytes.contains(&b'=') || bytes.contains(&0);
    #[cfg(windows)]
    let invalid = bytes.is_empty() || bytes[1..].contains(&b'=') || bytes.contains(&0);
    if invalid {
        Err(Error::invalid("spawn"))
    } else {
        Ok(())
    }
}
