pub mod constants;
pub mod engine;

pub const CODESIZE: usize = 4;

#[cfg(target_pointer_width = "32")]
pub const MAXREPEAT: usize = usize::MAX - 1;
#[cfg(target_pointer_width = "64")]
pub const MAXREPEAT: usize = u32::MAX as usize;

#[cfg(target_pointer_width = "32")]
pub const MAXGROUPS: usize = MAXREPEAT / 4 / 2;
#[cfg(target_pointer_width = "64")]
pub const MAXGROUPS: usize = MAXREPEAT / 2;
