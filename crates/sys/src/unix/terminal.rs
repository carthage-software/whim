use crate::{Descriptor, Error, Result};
use rustix::{io::Errno, termios};

pub fn is_terminal(descriptor: &Descriptor) -> bool {
    termios::isatty(descriptor.borrowed())
}

pub fn path(descriptor: &Descriptor) -> Result<Option<Vec<u8>>> {
    match termios::ttyname(descriptor.borrowed(), Vec::new()) {
        Ok(path) => Ok(Some(path.into_bytes())),
        Err(Errno::NOTTY) => Ok(None),
        Err(error) => Err(Error::new("ttyname_r", error)),
    }
}

pub fn size(descriptor: &Descriptor) -> Result<Option<(u16, u16)>> {
    match termios::tcgetwinsize(descriptor.borrowed()) {
        Ok(size) if size.ws_col == 0 || size.ws_row == 0 => Ok(None),
        Ok(size) => Ok(Some((size.ws_col, size.ws_row))),
        Err(Errno::NOTTY) => Ok(None),
        Err(error) => Err(Error::new("ioctl", error)),
    }
}
