use crate::VirtualMachine;
use libloading::Library;
use rustpython_common::lock::{PyMutex, PyRwLock};
use std::collections::HashMap;
use std::ffi::c_void;
use std::fmt;
use std::ptr::null;

pub struct SharedLibrary {
    pub(crate) lib: PyMutex<Option<Library>>,
}

impl fmt::Debug for SharedLibrary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SharedLibrary")
    }
}

impl SharedLibrary {
    pub fn new(name: &str) -> Result<SharedLibrary, libloading::Error> {
        Ok(SharedLibrary {
            lib: PyMutex::new(unsafe { Some(Library::new(name)?) }),
        })
    }

    pub fn get_pointer(&self) -> usize {
        let lib_lock = self.lib.lock();
        if let Some(l) = &*lib_lock {
            l as *const Library as usize
        } else {
            null::<c_void>() as usize
        }
    }

    pub fn is_closed(&self) -> bool {
        let lib_lock = self.lib.lock();
        lib_lock.is_none()
    }

    pub fn close(&self) {
        *self.lib.lock() = None;
    }
}

impl Drop for SharedLibrary {
    fn drop(&mut self) {
        self.close();
    }
}

pub struct ExternalLibs {
    libraries: HashMap<usize, SharedLibrary>,
}

impl ExternalLibs {
    pub fn new() -> Self {
        Self {
            libraries: HashMap::new(),
        }
    }

    #[allow(dead_code)]
    pub fn get_lib(&self, key: usize) -> Option<&SharedLibrary> {
        self.libraries.get(&key)
    }

    pub fn get_or_insert_lib(
        &mut self,
        library_path: &str,
        _vm: &VirtualMachine,
    ) -> Result<(usize, &SharedLibrary), libloading::Error> {
        let nlib = SharedLibrary::new(library_path)?;
        let key = nlib.get_pointer();

        match self.libraries.get(&key) {
            Some(l) => {
                if l.is_closed() {
                    self.libraries.insert(key, nlib);
                }
            }
            _ => {
                self.libraries.insert(key, nlib);
            }
        };

        Ok((key, self.libraries.get(&key).unwrap()))
    }

    pub fn drop_lib(&mut self, key: usize) {
        self.libraries.remove(&key);
    }
}

rustpython_common::static_cell! {
    static LIBCACHE: PyRwLock<ExternalLibs>;
}

pub fn libcache() -> &'static PyRwLock<ExternalLibs> {
    LIBCACHE.get_or_init(|| PyRwLock::new(ExternalLibs::new()))
}
