use crate::VirtualMachine;
use libloading::Library;
use rustpython_common::lock::{PyMutex, PyRwLock};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt;

#[cfg(unix)]
use libloading::os::unix::Library as OsLibrary;
#[cfg(windows)]
use libloading::os::windows::Library as OsLibrary;

pub struct SharedLibrary {
    pub(crate) lib: PyMutex<Option<Library>>,
}

impl fmt::Debug for SharedLibrary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SharedLibrary")
    }
}

impl SharedLibrary {
    #[cfg(windows)]
    pub fn new(name: impl AsRef<OsStr>) -> Result<SharedLibrary, libloading::Error> {
        Ok(SharedLibrary {
            lib: PyMutex::new(unsafe { Some(Library::new(name.as_ref())?) }),
        })
    }

    #[cfg(unix)]
    pub fn new_with_mode(
        name: impl AsRef<OsStr>,
        mode: i32,
    ) -> Result<SharedLibrary, libloading::Error> {
        Ok(SharedLibrary {
            lib: PyMutex::new(Some(unsafe {
                OsLibrary::open(Some(name.as_ref()), mode)?.into()
            })),
        })
    }

    /// Create a SharedLibrary from a raw dlopen handle (for pythonapi / dlopen(NULL))
    #[cfg(unix)]
    pub fn from_raw_handle(handle: *mut libc::c_void) -> SharedLibrary {
        SharedLibrary {
            lib: PyMutex::new(Some(unsafe { OsLibrary::from_raw(handle).into() })),
        }
    }

    /// Get the underlying OS handle (HMODULE on Windows, dlopen handle on Unix)
    #[cfg(unix)]
    pub fn get_pointer(&self) -> usize {
        let mut lib_lock = self.lib.lock();
        if let Some(lib) = lib_lock.take() {
            // Use official libloading API: convert to platform-specific type,
            // extract raw handle, then reconstruct
            let unix_lib: OsLibrary = lib.into();
            let handle = unix_lib.into_raw();
            // Reconstruct the library from the raw handle and put it back
            *lib_lock = Some(unsafe { OsLibrary::from_raw(handle) }.into());
            handle as usize
        } else {
            0
        }
    }

    /// Get the underlying OS handle (HMODULE on Windows, dlopen handle on Unix)
    #[cfg(windows)]
    pub fn get_pointer(&self) -> usize {
        let mut lib_lock = self.lib.lock();
        if let Some(lib) = lib_lock.take() {
            // Use official libloading API: convert to platform-specific type,
            // extract raw handle, then reconstruct
            let win_lib: OsLibrary = lib.into();
            let handle = win_lib.into_raw();
            // Reconstruct the library from the raw handle and put it back
            *lib_lock = Some(unsafe { OsLibrary::from_raw(handle) }.into());
            handle as usize
        } else {
            0
        }
    }

    fn is_closed(&self) -> bool {
        let lib_lock = self.lib.lock();
        lib_lock.is_none()
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

    #[cfg(windows)]
    pub fn get_or_insert_lib(
        &mut self,
        library_path: impl AsRef<OsStr>,
        _vm: &VirtualMachine,
    ) -> Result<(usize, &SharedLibrary), libloading::Error> {
        let new_lib = SharedLibrary::new(library_path)?;
        let key = new_lib.get_pointer();

        // Check if library already exists and is not closed
        let should_use_cached = self.libraries.get(&key).is_some_and(|l| !l.is_closed());

        if should_use_cached {
            // new_lib will be dropped, calling FreeLibrary (decrements refcount)
            // But library stays loaded because cached version maintains refcount
            drop(new_lib);
            return Ok((key, self.libraries.get(&key).expect("just checked")));
        }

        self.libraries.insert(key, new_lib);
        Ok((key, self.libraries.get(&key).expect("just inserted")))
    }

    #[cfg(unix)]
    pub fn get_or_insert_lib_with_mode(
        &mut self,
        library_path: impl AsRef<OsStr>,
        mode: i32,
        _vm: &VirtualMachine,
    ) -> Result<(usize, &SharedLibrary), libloading::Error> {
        let new_lib = SharedLibrary::new_with_mode(library_path, mode)?;
        let key = new_lib.get_pointer();

        // Check if library already exists and is not closed
        let should_use_cached = self.libraries.get(&key).is_some_and(|l| !l.is_closed());

        if should_use_cached {
            // new_lib will be dropped, calling dlclose (decrements refcount)
            // But library stays loaded because cached version maintains refcount
            drop(new_lib);
            return Ok((key, self.libraries.get(&key).expect("just checked")));
        }

        self.libraries.insert(key, new_lib);
        Ok((key, self.libraries.get(&key).expect("just inserted")))
    }

    /// Insert a raw dlopen handle into the cache (for pythonapi / dlopen(NULL))
    #[cfg(unix)]
    pub fn insert_raw_handle(&mut self, handle: *mut libc::c_void) -> usize {
        let shared_lib = SharedLibrary::from_raw_handle(handle);
        let key = handle as usize;
        self.libraries.insert(key, shared_lib);
        key
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
