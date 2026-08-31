//! VM-independent compression engines shared by RustPython components.

#[cfg(all(
    feature = "lzma",
    not(any(target_os = "android", target_arch = "wasm32"))
))]
pub mod lzma;
#[cfg(feature = "zlib")]
pub mod zlib;
