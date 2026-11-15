//! Lazy loading of the Unicode name database used by the codecs.
// spell-checker:ignore ucnhash

use core::sync::atomic::Ordering;

use crate::{PyRef, PyResult, VirtualMachine, builtins::PyCapsule};

const CAPSULE_NAME: &core::ffi::CStr = c"unicodedata._ucnhash_CAPI";

// The capsule pointer identifies the bundled name database and is never
// dereferenced.
static NAME_DATABASE: u8 = 0;

impl VirtualMachine {
    /// Create the native module's private Unicode-name capsule.
    pub fn new_unicode_name_capsule(&self) -> PyRef<PyCapsule> {
        self.ctx.new_capsule(
            core::ptr::from_ref(&NAME_DATABASE).cast_mut().cast(),
            Some(CAPSULE_NAME),
            None,
        )
    }

    pub(crate) fn load_unicode_names(&self) -> PyResult<()> {
        if self.state.unicode_names_loaded.load(Ordering::Acquire) {
            return Ok(());
        }
        // Do not hold an initialization lock across import: import can reenter
        // the interpreter. Failed imports must not poison the successful cache.
        let module = self.import("unicodedata", 0).map_err(|_| {
            self.new_exception_msg(
                self.ctx.exceptions.import_error.to_owned(),
                "PyCapsule_Import could not import module \"unicodedata\"".into(),
            )
        })?;
        let object = module.get_attr("_ucnhash_CAPI", self)?;
        let valid = object
            .downcast_ref_if_exact::<PyCapsule>(self)
            .is_some_and(|capsule| {
                capsule.name() == Some(CAPSULE_NAME)
                    && core::ptr::eq(
                        capsule.pointer(),
                        core::ptr::from_ref(&NAME_DATABASE).cast_mut().cast(),
                    )
            });
        if !valid {
            return Err(self.new_attribute_error(
                "PyCapsule_Import \"unicodedata._ucnhash_CAPI\" is not valid",
            ));
        }
        self.state
            .unicode_names_loaded
            .store(true, Ordering::Release);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Interpreter;

    #[test]
    fn missing_unicodedata_is_an_error() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let modules = vm.sys_module.get_attr("modules", vm).unwrap();
            modules.set_item("unicodedata", vm.ctx.none(), vm).unwrap();
            assert!(vm.load_unicode_names().is_err());
        });
    }

    #[test]
    fn foreign_pointer_is_rejected_and_not_cached() {
        static FOREIGN: u8 = 0;
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let modules = vm.sys_module.get_attr("modules", vm).unwrap();
            let dict = vm.ctx.new_dict();
            let capsule = vm.ctx.new_capsule(
                core::ptr::from_ref(&FOREIGN).cast_mut().cast(),
                Some(CAPSULE_NAME),
                None,
            );
            dict.set_item("_ucnhash_CAPI", capsule.into(), vm).unwrap();
            let module = vm.new_module("unicodedata", dict, None);
            modules.set_item("unicodedata", module.into(), vm).unwrap();
            assert!(vm.load_unicode_names().is_err());

            let dict = vm.ctx.new_dict();
            dict.set_item("_ucnhash_CAPI", vm.new_unicode_name_capsule().into(), vm)
                .unwrap();
            let module = vm.new_module("unicodedata", dict, None);
            modules.set_item("unicodedata", module.into(), vm).unwrap();
            assert!(vm.load_unicode_names().is_ok());
        });
    }

    #[test]
    fn successful_load_is_cached_after_module_removal() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let modules = vm.sys_module.get_attr("modules", vm).unwrap();
            let dict = vm.ctx.new_dict();
            dict.set_item("_ucnhash_CAPI", vm.new_unicode_name_capsule().into(), vm)
                .unwrap();
            let module = vm.new_module("unicodedata", dict, None);
            modules.set_item("unicodedata", module.into(), vm).unwrap();
            assert!(vm.load_unicode_names().is_ok());
            modules.del_item("unicodedata", vm).unwrap();
            assert!(vm.load_unicode_names().is_ok());
        });
    }

    #[test]
    fn cache_is_interpreter_local() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let modules = vm.sys_module.get_attr("modules", vm).unwrap();
            let dict = vm.ctx.new_dict();
            dict.set_item("_ucnhash_CAPI", vm.new_unicode_name_capsule().into(), vm)
                .unwrap();
            let module = vm.new_module("unicodedata", dict, None);
            modules.set_item("unicodedata", module.into(), vm).unwrap();
            assert!(vm.load_unicode_names().is_ok());
        });
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let modules = vm.sys_module.get_attr("modules", vm).unwrap();
            modules.set_item("unicodedata", vm.ctx.none(), vm).unwrap();
            assert!(vm.load_unicode_names().is_err());
        });
    }
}
