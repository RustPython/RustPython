use pvm_host::HostApi;
use std::cell::Cell;
use std::marker::PhantomData;
use std::mem;

type HostPtr = *mut (dyn HostApi + 'static);

thread_local! {
    static HOST: Cell<Option<HostPtr>> = Cell::new(None);
}

pub struct HostGuard<'a> {
    _marker: PhantomData<&'a mut dyn HostApi>,
}

impl<'a> HostGuard<'a> {
    pub fn install(host: &'a mut dyn HostApi) -> Self {
        let ptr = host as *mut dyn HostApi;
        // Erase the lifetime; the guard ensures the pointer is only used in-scope.
        let ptr = unsafe { mem::transmute::<*mut dyn HostApi, HostPtr>(ptr) };
        HOST.with(|cell| cell.set(Some(ptr)));
        Self {
            _marker: PhantomData,
        }
    }
}

impl Drop for HostGuard<'_> {
    fn drop(&mut self) {
        HOST.with(|cell| cell.set(None));
    }
}

pub(crate) fn with_host<R>(f: impl FnOnce(&mut dyn HostApi) -> R) -> Option<R> {
    HOST.with(|cell| {
        let ptr = cell.get()?;
        // Safety: host pointer is installed for the duration of an execution.
        Some(unsafe { f(&mut *ptr) })
    })
}
