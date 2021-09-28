#[cfg(windows)]
pub use super::stdlib::nt::module as nt;
#[cfg(unix)]
pub use super::stdlib::posix::module as posix;
pub use super::stdlib::{pystruct::_struct, time::time};
