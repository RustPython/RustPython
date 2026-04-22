#![allow(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "These helpers are thin wrappers around raw Windows mapping APIs."
)]

use std::io;

#[cfg(unix)]
use crate::{crt_fd, fileutils, posix};
use memmap2::{Mmap, MmapMut, MmapOptions};
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, GetLastError, HANDLE,
        INVALID_HANDLE_VALUE,
    },
    Storage::FileSystem::{FILE_BEGIN, GetFileSize, SetEndOfFile, SetFilePointerEx},
    System::{
        Memory::{
            CreateFileMappingW, FILE_MAP_COPY, FILE_MAP_READ, FILE_MAP_WRITE, FlushViewOfFile,
            MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, PAGE_READONLY, PAGE_READWRITE,
            PAGE_WRITECOPY, UnmapViewOfFile,
        },
        Threading::GetCurrentProcess,
    },
};

#[cfg(windows)]
pub type Handle = HANDLE;
#[cfg(windows)]
pub const INVALID_HANDLE: Handle = INVALID_HANDLE_VALUE;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessMode {
    Default = 0,
    Read = 1,
    Write = 2,
    Copy = 3,
}

#[cfg(windows)]
#[derive(Debug)]
pub struct NamedMmap {
    map_handle: Handle,
    view_ptr: *mut u8,
    len: usize,
}

#[derive(Debug)]
pub enum MappedFile {
    Read(Mmap),
    Write(MmapMut),
}

impl MappedFile {
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Read(mmap) => &mmap[..],
            Self::Write(mmap) => &mmap[..],
        }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        match self {
            Self::Read(_) => panic!("mmap can't modify a readonly memory map."),
            Self::Write(mmap) => &mut mmap[..],
        }
    }

    pub fn as_ptr(&self) -> *const u8 {
        match self {
            Self::Read(mmap) => mmap.as_ptr(),
            Self::Write(mmap) => mmap.as_ptr(),
        }
    }

    pub fn flush_range(&self, offset: usize, size: usize) -> io::Result<()> {
        match self {
            Self::Read(_) => Ok(()),
            Self::Write(mmap) => mmap.flush_range(offset, size),
        }
    }

    #[cfg(all(unix, not(target_os = "redox")))]
    pub fn madvise_range(&self, start: usize, length: usize, advice: i32) -> io::Result<()> {
        let ptr = unsafe { self.as_ptr().add(start) };
        posix::madvise(ptr as usize, length, advice)
    }
}

#[cfg(windows)]
unsafe impl Send for NamedMmap {}
#[cfg(windows)]
unsafe impl Sync for NamedMmap {}

#[cfg(windows)]
impl NamedMmap {
    pub fn as_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.view_ptr, self.len) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.view_ptr, self.len) }
    }

    pub fn ptr_at(&self, offset: usize) -> *const core::ffi::c_void {
        unsafe { self.view_ptr.add(offset) as *const _ }
    }

    pub fn flush_range(&self, offset: usize, size: usize) -> io::Result<()> {
        flush_view(self.ptr_at(offset), size)
    }
}

#[cfg(windows)]
impl Drop for NamedMmap {
    fn drop(&mut self) {
        unsafe {
            if !self.view_ptr.is_null() {
                UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: self.view_ptr as *mut _,
                });
            }
            if !self.map_handle.is_null() {
                CloseHandle(self.map_handle);
            }
        }
    }
}

#[cfg(windows)]
pub fn duplicate_handle(handle: Handle) -> io::Result<Handle> {
    let mut new_handle: Handle = INVALID_HANDLE;
    let result = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            handle,
            GetCurrentProcess(),
            &mut new_handle,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(new_handle)
    }
}

#[cfg(windows)]
pub fn get_file_len(handle: Handle) -> io::Result<i64> {
    let mut high: u32 = 0;
    let low = unsafe { GetFileSize(handle, &mut high) };
    if low == u32::MAX {
        let err = io::Error::last_os_error();
        if err.raw_os_error() != Some(0) {
            return Err(err);
        }
    }
    Ok(((high as i64) << 32) | (low as i64))
}

#[cfg(unix)]
pub fn file_len(fd: crt_fd::Borrowed<'_>) -> io::Result<i64> {
    Ok(fileutils::fstat(fd)?.st_size)
}

#[cfg(unix)]
pub fn prepare_file_mapping(fd: crt_fd::Borrowed<'_>) {
    #[cfg(target_os = "macos")]
    {
        let _ = posix::full_fsync(fd.into());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = fd;
    }
}

#[cfg(windows)]
pub fn is_invalid_handle_value(handle: isize) -> bool {
    handle == INVALID_HANDLE as isize
}

#[cfg(windows)]
pub fn extend_file(handle: Handle, size: i64) -> io::Result<()> {
    if unsafe { SetFilePointerEx(handle, size, core::ptr::null_mut(), FILE_BEGIN) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { SetEndOfFile(handle) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
pub fn close_descriptor(fd: i32) {
    if fd >= 0 {
        let _ = crt_fd::close(unsafe { crt_fd::Owned::from_raw(fd) });
    }
}

#[cfg(windows)]
pub fn close_handle(handle: Handle) {
    unsafe { CloseHandle(handle) };
}

#[cfg(windows)]
pub fn flush_view(ptr: *const core::ffi::c_void, size: usize) -> io::Result<()> {
    if unsafe { FlushViewOfFile(ptr, size) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
pub fn last_error() -> u32 {
    unsafe { GetLastError() }
}

#[cfg(windows)]
pub fn create_named_mapping(
    file_handle: Handle,
    tag: &str,
    access: AccessMode,
    offset: i64,
    map_size: usize,
) -> io::Result<NamedMmap> {
    let (fl_protect, desired_access) = match access {
        AccessMode::Default | AccessMode::Write => (PAGE_READWRITE, FILE_MAP_WRITE),
        AccessMode::Read => (PAGE_READONLY, FILE_MAP_READ),
        AccessMode::Copy => (PAGE_WRITECOPY, FILE_MAP_COPY),
    };

    let total_size = (offset as u64)
        .checked_add(map_size as u64)
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EOVERFLOW))?;
    let size_hi = (total_size >> 32) as u32;
    let size_lo = total_size as u32;
    let tag_wide: Vec<u16> = tag.encode_utf16().chain(core::iter::once(0)).collect();

    let map_handle = unsafe {
        CreateFileMappingW(
            file_handle,
            core::ptr::null(),
            fl_protect,
            size_hi,
            size_lo,
            tag_wide.as_ptr(),
        )
    };
    if map_handle.is_null() {
        return Err(io::Error::last_os_error());
    }

    let off_hi = (offset as u64 >> 32) as u32;
    let off_lo = offset as u32;
    let view = unsafe { MapViewOfFile(map_handle, desired_access, off_hi, off_lo, map_size) };
    if view.Value.is_null() {
        unsafe { CloseHandle(map_handle) };
        return Err(io::Error::last_os_error());
    }

    Ok(NamedMmap {
        map_handle,
        view_ptr: view.Value as *mut u8,
        len: map_size,
    })
}

#[cfg(unix)]
pub fn map_anon(size: usize) -> io::Result<MappedFile> {
    let mut mmap_opt = MmapOptions::new();
    mmap_opt.len(size).map_anon().map(MappedFile::Write)
}

#[cfg(windows)]
pub fn map_anon(size: usize) -> io::Result<MappedFile> {
    let mut mmap_opt = MmapOptions::new();
    mmap_opt.len(size).map_anon().map(MappedFile::Write)
}

#[cfg(unix)]
pub fn map_file(
    fd: crt_fd::Borrowed<'_>,
    offset: i64,
    size: usize,
    access: AccessMode,
) -> io::Result<(crt_fd::Owned, MappedFile)> {
    let new_fd: crt_fd::Owned = posix::dup_noninheritable(fd.into())?.into();
    let mut mmap_opt = MmapOptions::new();
    let mmap_opt = mmap_opt.offset(offset as u64).len(size);

    let mapped = match access {
        AccessMode::Default | AccessMode::Write => {
            unsafe { mmap_opt.map_mut(&new_fd) }.map(MappedFile::Write)?
        }
        AccessMode::Read => unsafe { mmap_opt.map(&new_fd) }.map(MappedFile::Read)?,
        AccessMode::Copy => unsafe { mmap_opt.map_copy(&new_fd) }.map(MappedFile::Write)?,
    };

    Ok((new_fd, mapped))
}

#[cfg(all(unix, not(target_os = "redox")))]
pub fn validate_advice(advice: i32) -> bool {
    match advice {
        libc::MADV_NORMAL
        | libc::MADV_RANDOM
        | libc::MADV_SEQUENTIAL
        | libc::MADV_WILLNEED
        | libc::MADV_DONTNEED => true,
        #[cfg(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd"
        ))]
        libc::MADV_FREE => true,
        #[cfg(target_os = "linux")]
        libc::MADV_DONTFORK
        | libc::MADV_DOFORK
        | libc::MADV_MERGEABLE
        | libc::MADV_UNMERGEABLE
        | libc::MADV_HUGEPAGE
        | libc::MADV_NOHUGEPAGE
        | libc::MADV_REMOVE
        | libc::MADV_DONTDUMP
        | libc::MADV_DODUMP
        | libc::MADV_HWPOISON => true,
        #[cfg(target_os = "freebsd")]
        libc::MADV_NOSYNC
        | libc::MADV_AUTOSYNC
        | libc::MADV_NOCORE
        | libc::MADV_CORE
        | libc::MADV_PROTECT => true,
        _ => false,
    }
}

#[cfg(windows)]
pub fn map_handle(
    handle: Handle,
    offset: i64,
    size: usize,
    access: AccessMode,
) -> io::Result<MappedFile> {
    use std::{
        fs::File,
        os::windows::io::{FromRawHandle, RawHandle},
    };

    let file = unsafe { File::from_raw_handle(handle as RawHandle) };
    let mut mmap_opt = MmapOptions::new();
    let mmap_opt = mmap_opt.offset(offset as u64).len(size);

    let result = match access {
        AccessMode::Default | AccessMode::Write => {
            unsafe { mmap_opt.map_mut(&file) }.map(MappedFile::Write)
        }
        AccessMode::Read => unsafe { mmap_opt.map(&file) }.map(MappedFile::Read),
        AccessMode::Copy => unsafe { mmap_opt.map_copy(&file) }.map(MappedFile::Write),
    };

    core::mem::forget(file);
    result
}
