// cspell:ignore iovecs iovs nwritten fdflags fdstat
//! Wasmer bindings for the shared capability-scoped WASI host.
use super::Context;
use rustpython_wasi_host::guest_memory::{self, GuestMemory};
pub(crate) use rustpython_wasi_host::{FUNCTIONS, MODULE, Preopen, Table};
use wasmer::{FunctionEnvMut, MemoryView};

struct Memory<'a>(Option<MemoryView<'a>>);
impl GuestMemory for Memory<'_> {
    fn validate(&self) -> Result<(), u16> {
        self.0.as_ref().map(|_| ()).ok_or(rustpython_wasi_host::IO)
    }
    fn size(&self) -> u64 {
        self.0.as_ref().map_or(0, MemoryView::data_size)
    }
    fn read(&self, offset: u32, bytes: &mut [u8]) -> Result<(), u16> {
        self.0
            .as_ref()
            .ok_or(rustpython_wasi_host::IO)?
            .read(u64::from(offset), bytes)
            .map_err(|_| rustpython_wasi_host::FAULT)
    }
    fn write(&self, offset: u32, bytes: &[u8]) -> Result<(), u16> {
        self.0
            .as_ref()
            .ok_or(rustpython_wasi_host::IO)?
            .write(u64::from(offset), bytes)
            .map_err(|_| rustpython_wasi_host::FAULT)
    }
}

pub(crate) fn fd_read(
    mut ctx: FunctionEnvMut<Context>,
    fd: i32,
    iovs: i32,
    iovs_len: i32,
    nread: i32,
) -> i32 {
    let (context, store) = ctx.data_and_store_mut();
    let memory = context.memory.clone();
    let view = Memory(memory.as_ref().map(|m| m.view(&store)));
    guest_memory::fd_read(&mut context.table, &view, fd, iovs, iovs_len, nread)
}

pub(crate) fn fd_write(
    mut ctx: FunctionEnvMut<Context>,
    fd: i32,
    iovs: i32,
    iovs_len: i32,
    nwritten: i32,
) -> i32 {
    let (context, store) = ctx.data_and_store_mut();
    let memory = context.memory.clone();
    let view = Memory(memory.as_ref().map(|m| m.view(&store)));
    guest_memory::fd_write(&mut context.table, &view, fd, iovs, iovs_len, nwritten)
}

pub(crate) fn fd_seek(
    mut ctx: FunctionEnvMut<Context>,
    fd: i32,
    offset: i64,
    whence: i32,
    newoffset: i32,
) -> i32 {
    let (context, store) = ctx.data_and_store_mut();
    let memory = context.memory.clone();
    let view = Memory(memory.as_ref().map(|m| m.view(&store)));
    guest_memory::fd_seek(&mut context.table, &view, fd, offset, whence, newoffset)
}

pub(crate) fn fd_close(mut ctx: FunctionEnvMut<Context>, fd: i32) -> i32 {
    guest_memory::fd_close(&mut ctx.data_mut().table, fd)
}

pub(crate) fn fd_sync(mut ctx: FunctionEnvMut<Context>, fd: i32) -> i32 {
    guest_memory::fd_sync(&mut ctx.data_mut().table, fd)
}

pub(crate) fn fd_filestat_set_size(mut ctx: FunctionEnvMut<Context>, fd: i32, size: i64) -> i32 {
    guest_memory::fd_filestat_set_size(&mut ctx.data_mut().table, fd, size)
}

pub(crate) fn fd_fdstat_get(mut ctx: FunctionEnvMut<Context>, fd: i32, buf: i32) -> i32 {
    let (context, store) = ctx.data_and_store_mut();
    let memory = context.memory.clone();
    let view = Memory(memory.as_ref().map(|m| m.view(&store)));
    guest_memory::fd_fdstat_get(&mut context.table, &view, fd, buf)
}

pub(crate) fn fd_prestat_get(mut ctx: FunctionEnvMut<Context>, fd: i32, buf: i32) -> i32 {
    let (context, store) = ctx.data_and_store_mut();
    let memory = context.memory.clone();
    let view = Memory(memory.as_ref().map(|m| m.view(&store)));
    guest_memory::fd_prestat_get(&mut context.table, &view, fd, buf)
}

pub(crate) fn fd_prestat_dir_name(
    mut ctx: FunctionEnvMut<Context>,
    fd: i32,
    path: i32,
    path_len: i32,
) -> i32 {
    let (context, store) = ctx.data_and_store_mut();
    let memory = context.memory.clone();
    let view = Memory(memory.as_ref().map(|m| m.view(&store)));
    guest_memory::fd_prestat_dir_name(&mut context.table, &view, fd, path, path_len)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn path_open(
    mut ctx: FunctionEnvMut<Context>,
    dirfd: i32,
    dirflags: i32,
    path: i32,
    path_len: i32,
    oflags: i32,
    fs_rights_base: i64,
    fs_rights_inheriting: i64,
    fdflags: i32,
    fd: i32,
) -> i32 {
    let (context, store) = ctx.data_and_store_mut();
    let memory = context.memory.clone();
    let view = Memory(memory.as_ref().map(|m| m.view(&store)));
    guest_memory::path_open(
        &mut context.table,
        &view,
        dirfd,
        dirflags,
        path,
        path_len,
        oflags,
        fs_rights_base,
        fs_rights_inheriting,
        fdflags,
        fd,
    )
}
