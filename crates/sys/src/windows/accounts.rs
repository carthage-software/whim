use crate::accounts::{Group, Identity, User};
use crate::{Error, Result};
use std::ffi::CString;

pub fn require_support() -> Result<()> {
    Err(Error::Unsupported("POSIX account lookup"))
}
pub fn user(_identity: &Identity) -> Result<Option<User>> {
    Err(Error::Unsupported("POSIX user lookup"))
}
pub fn group(_identity: &Identity) -> Result<Option<Group>> {
    Err(Error::Unsupported("POSIX group lookup"))
}
pub fn groups_for_user(_name: &CString, _primary_group: u32) -> Result<Vec<Group>> {
    Err(Error::Unsupported("POSIX group lookup"))
}
