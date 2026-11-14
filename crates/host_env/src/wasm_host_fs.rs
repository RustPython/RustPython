//! Read-only path opening relative to an explicitly preopened host root.
//! Paths contain Unix filename bytes; Windows hosts initially require UTF-8.
//! The host validates memory before allocating and independently rejects flags
//! other than zero. Success transfers a descriptor; errors are negative guest
//! errno values. No stat, enumeration, or ambient path authority is provided.
#[link(wasm_import_module = "rustpython_host_fs_v1")]
unsafe extern "C" {
    pub(super) fn path_open(ptr: u32, len: u32, flags: i32) -> i64;
}
