use crate::VirtualMachine;
use libloading::Library;
use rustpython_common::lock::{PyMutex, PyRwLock};
use std::collections::HashMap;
use std::ffi::{OsStr, c_void};
use std::fmt;
use std::ptr::null;

pub(super) struct SharedLibrary {
    pub(super) lib: PyMutex<Option<Library>>,
}

impl fmt::Debug for SharedLibrary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SharedLibrary")
    }
}

impl SharedLibrary {
    fn new(name: impl AsRef<OsStr>) -> Result<SharedLibrary, libloading::Error> {
        Ok(SharedLibrary {
            lib: PyMutex::new(unsafe { Some(Library::new(name.as_ref())?) }),
        })
    }

    fn get_pointer(&self) -> usize {
        let lib_lock = self.lib.lock();
        if let Some(l) = &*lib_lock {
            l as *const Library as usize
        } else {
            null::<c_void>() as usize
        }
    }

    fn is_closed(&self) -> bool {
        let lib_lock = self.lib.lock();
        lib_lock.is_none()
    }

    fn close(&self) {
        *self.lib.lock() = None;
    }
}

impl Drop for SharedLibrary {
    fn drop(&mut self) {
        self.close();
    }
}

pub(super) struct ExternalLibs {
    libraries: HashMap<usize, SharedLibrary>,
}

impl ExternalLibs {
    fn new() -> Self {
        Self {
            libraries: HashMap::new(),
        }
    }

    pub fn get_lib(&self, key: usize) -> Option<&SharedLibrary> {
        self.libraries.get(&key)
    }

    pub fn get_or_insert_lib(
        &mut self,
        library_path: impl AsRef<OsStr>,
        _vm: &VirtualMachine,
    ) -> Result<(usize, &SharedLibrary), libloading::Error> {
        let new_lib = SharedLibrary::new(library_path)?;
        let key = new_lib.get_pointer();

        match self.libraries.get(&key) {
            Some(l) => {
                if l.is_closed() {
                    self.libraries.insert(key, new_lib);
                }
            }
            _ => {
                self.libraries.insert(key, new_lib);
            }
        };

        Ok((key, self.libraries.get(&key).expect("just inserted")))
    }

    pub fn drop_lib(&mut self, key: usize) {
        self.libraries.remove(&key);
    }
}

pub(super) fn libcache() -> &'static PyRwLock<ExternalLibs> {
    rustpython_common::static_cell! {
        static LIBCACHE: PyRwLock<ExternalLibs>;
    }
    LIBCACHE.get_or_init(|| PyRwLock::new(ExternalLibs::new()))
}
