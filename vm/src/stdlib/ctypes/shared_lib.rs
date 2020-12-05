extern crate lazy_static;
extern crate libffi;
extern crate libloading;

use ::std::{collections::HashMap, fmt, os::raw::c_void, ptr::null};

use crossbeam_utils::atomic::AtomicCell;
use libloading::Library;

use crate::builtins::PyTypeRef;
use crate::common::lock::PyRwLock;
use crate::pyobject::{PyRef, PyValue};
use crate::VirtualMachine;
#[pyclass(module = false, name = "SharedLibrary")]
pub struct SharedLibrary {
    path_name: String,
    lib: AtomicCell<Option<Library>>,
}

impl fmt::Debug for SharedLibrary {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "SharedLibrary {{
            path_name: {},
            lib: {},
        }}",
            self.path_name.as_str(),
            self.get_pointer()
        )
    }
}

impl PyValue for SharedLibrary {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.object_type
    }
}

impl SharedLibrary {
    pub fn new(name: &str) -> Result<SharedLibrary, libloading::Error> {
        Ok(SharedLibrary {
            path_name: name.to_string(),
            lib: AtomicCell::new(Some(Library::new(name.to_string())?)),
        })
    }

    pub fn get_sym(&self, name: &str) -> Result<*mut c_void, String> {
        let inner = if let Some(ref inner) = unsafe { &*self.lib.as_ptr() } {
            inner
        } else {
            return Err("The library has been closed".to_string());
        };

        unsafe {
            inner
                .get(name.as_bytes())
                .map(|f: libloading::Symbol<*mut c_void>| *f)
                .map_err(|err| err.to_string())
        }
    }

    pub fn get_pointer(&self) -> usize {
        if let Some(l) = unsafe { &*self.lib.as_ptr() } {
            l as *const Library as *const u32 as usize
        } else {
            null() as *const u32 as usize
        }
    }

    pub fn is_closed(&self) -> bool {
        unsafe { &*self.lib.as_ptr() }.is_none()
    }

    pub fn close(&self) {
        let old = self.lib.take();
        self.lib.store(None);
        drop(old);
    }
}

#[pyimpl]
impl SharedLibrary {
    #[pymethod(magic)]
    fn repr(&self) -> String {
        let pointer = self.get_pointer();

        let value = if pointer == 0usize {
            "None".to_string()
        } else {
            pointer.to_string()
        };

        format!("SharedLibrary({})", value)
    }
}

pub struct ExternalLibs {
    libraries: HashMap<String, PyRef<SharedLibrary>>,
}

impl ExternalLibs {
    pub fn new() -> Self {
        Self {
            libraries: HashMap::new(),
        }
    }

    pub fn get_or_insert_lib(
        &mut self,
        library_path: &str,
        vm: &VirtualMachine,
    ) -> Result<&PyRef<SharedLibrary>, libloading::Error> {
        match self.libraries.get(&library_path.to_string()) {
            Some(l) => {
                if l.is_closed() {
                    self.libraries.insert(
                        library_path.to_string(),
                        SharedLibrary::new(library_path)?.into_ref(vm),
                    );
                }
            }
            _ => {
                self.libraries.insert(
                    library_path.to_string(),
                    SharedLibrary::new(library_path)?.into_ref(vm),
                );
            }
        };

        Ok(self.libraries.get(&library_path.to_string()).unwrap())
    }
}

lazy_static::lazy_static! {
    pub static ref LIBCACHE: PyRwLock<ExternalLibs> = PyRwLock::new(ExternalLibs::new());
}
