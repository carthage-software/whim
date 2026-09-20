pub(crate) mod accounts;
pub mod constants;
mod descriptor;
pub(crate) mod file;
pub(crate) mod filesystem;
pub(crate) mod message;
pub(crate) mod process;
pub(crate) mod socket;
pub mod system;
pub mod terminal;

pub use descriptor::Descriptor;

pub fn initialize_console() {}
