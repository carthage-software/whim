use std::io;
use std::io::Write;

use crate::error::Error;

pub(crate) fn write(text: &str) -> Result<bool, Error> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    match output
        .write_all(text.as_bytes())
        .and_then(|()| output.flush())
    {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(error) => Err(Error::WriteOutput(error)),
    }
}

pub(crate) fn write_error(text: &str) -> Result<(), Error> {
    let stderr = io::stderr();
    let mut output = stderr.lock();
    output
        .write_all(text.as_bytes())
        .and_then(|()| output.write_all(b"\n"))
        .and_then(|()| output.flush())
        .map_err(Error::WriteErrorOutput)
}
