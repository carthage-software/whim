use std::ffi::CString;

pub use crate::platform::accounts::{group, groups_for_user, require_support, user};

pub enum Identity {
    Name(CString),
    Id(u32),
}

pub struct User {
    pub name: Vec<u8>,
    pub id: u32,
    pub primary_group: u32,
    pub home_directory: Vec<u8>,
    pub shell: Vec<u8>,
}

pub struct Group {
    pub name: Vec<u8>,
    pub id: u32,
    pub members: Vec<Vec<u8>>,
}
