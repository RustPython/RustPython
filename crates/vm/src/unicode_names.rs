//! Lazy access to the native Unicode name table.
// spell-checker:ignore ucnhash getucname

use core::ffi::{c_char, c_int};
use core::sync::atomic::Ordering;

use crate::{PyRef, PyResult, VirtualMachine, builtins::PyCapsule};

const CAPSULE_NAME: &core::ffi::CStr = c"unicodedata._ucnhash_CAPI";

// This is the private two-function table used by CPython's Unicode codecs.
// Static storage keeps it valid after unicodedata is removed from sys.modules.
#[repr(C)]
pub(crate) struct UnicodeNameCapi {
    getname: unsafe extern "C" fn(u32, *mut c_char, c_int, c_int) -> c_int,
    getcode: unsafe extern "C" fn(*const c_char, c_int, *mut u32, c_int) -> c_int,
}

unsafe extern "C" fn getname(
    code: u32,
    buffer: *mut c_char,
    length: c_int,
    with_alias_and_seq: c_int,
) -> c_int {
    let Some(name) = (if with_alias_and_seq != 0 {
        rustpython_unicode::data::name_for_namereplace(code)
    } else {
        char::from_u32(code).and_then(rustpython_unicode::character_name)
    }) else {
        return 0;
    };
    // Match CPython's _getucname buffer requirements: Hangul reserves its
    // worst-case size, derived hex names need the terminator, and inverse DAWG
    // names reject a buffer with only one byte left at the terminal node.
    if buffer.is_null() || length <= 0 || name.len() >= length as usize {
        return 0;
    }
    if name.starts_with("HANGUL SYLLABLE ") {
        if length < 27 {
            return 0;
        }
    } else if length as usize == name.len() + 1 && !name.ends_with(&format!("{code:04X}")) {
        // makeunicodedata derives every name ending in its own hex code, not
        // only the algorithmic First/Last ranges (including non-range names).
        return 0;
    }
    // SAFETY: The capsule API caller supplies a writable buffer of `length` bytes.
    unsafe {
        core::ptr::copy_nonoverlapping(name.as_ptr(), buffer.cast(), name.len());
        buffer.add(name.len()).write(0);
    }
    1
}

unsafe extern "C" fn getcode(
    name: *const c_char,
    length: c_int,
    code: *mut u32,
    with_named_seq: c_int,
) -> c_int {
    if name.is_null() || code.is_null() || length < 0 {
        return 0;
    }
    // SAFETY: The capsule API caller supplies `length` readable name bytes.
    let bytes = unsafe { core::slice::from_raw_parts(name.cast(), length as usize) };
    let Some(character) = core::str::from_utf8(bytes)
        .ok()
        .and_then(|name| rustpython_unicode::data::lookup_name_code(name, with_named_seq != 0))
    else {
        return 0;
    };
    // SAFETY: The capsule API caller supplies a writable code point pointer.
    unsafe { code.write(character) };
    1
}

static NAME_API: UnicodeNameCapi = UnicodeNameCapi { getname, getcode };

impl VirtualMachine {
    /// Create the native module's private Unicode-name capsule.
    pub fn new_unicode_name_capsule(&self) -> PyRef<PyCapsule> {
        // SAFETY: immutable table, callbacks and bundled data all have static
        // lifetime, including after the module or capsule has been released.
        unsafe {
            self.ctx.new_capsule(
                core::ptr::from_ref(&NAME_API).cast_mut().cast(),
                Some(CAPSULE_NAME),
                None,
            )
        }
    }

    pub(crate) fn unicode_names(&self) -> PyResult<&UnicodeNameCapi> {
        let cached = self.state.unicode_names.load(Ordering::Acquire);
        if !cached.is_null() {
            // SAFETY: the imported Unicode-name protocol requires immutable
            // table/code/data valid for the interpreter's entire lifetime.
            return Ok(unsafe { &*cached });
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
        let capsule = object
            .downcast_ref_if_exact::<PyCapsule>(self)
            .ok_or_else(|| {
                self.new_attribute_error(
                    "PyCapsule_Import \"unicodedata._ucnhash_CAPI\" is not valid",
                )
            })?;
        let table = capsule.pointer().cast::<UnicodeNameCapi>();
        if capsule.name() != Some(CAPSULE_NAME) || table.is_null() {
            return Err(self.new_attribute_error(
                "PyCapsule_Import \"unicodedata._ucnhash_CAPI\" is not valid",
            ));
        }
        // SAFETY: unsafe capsule creation/mutation requires its producer to
        // honor this named protocol: the exact two-callback layout, immutable
        // data and functions valid until every interpreter consumer has exited.
        // Name validation identifies that contract; it does not prove pointer
        // validity. As in CPython, no lock is held across the import above.
        self.state.unicode_names.store(table, Ordering::Release);
        Ok(unsafe { &*table })
    }
}

impl crate::common::encodings::UnicodeNames for UnicodeNameCapi {
    fn name<'a>(
        &self,
        code: u32,
        buffer: &'a mut [u8; 256],
    ) -> Option<alloc::borrow::Cow<'a, str>> {
        // SAFETY: this table came from the validated native protocol; buffer is
        // writable for the supplied length and getname must NUL-terminate.
        let found = unsafe { (self.getname)(code, buffer.as_mut_ptr().cast(), 256, 1) };
        if found == 0 {
            return None;
        }
        let end = buffer.iter().position(|&c| c == 0)?;
        core::str::from_utf8(&buffer[..end])
            .ok()
            .map(alloc::borrow::Cow::Borrowed)
    }

    fn character(&self, name: &[u8]) -> Option<crate::common::wtf8::CodePoint> {
        let length = c_int::try_from(name.len()).ok()?;
        let mut code = u32::MAX;
        // SAFETY: name and code are valid for the lengths the protocol expects.
        let found = unsafe { (self.getcode)(name.as_ptr().cast(), length, &mut code, 0) };
        if found == 0 {
            None
        } else {
            crate::common::wtf8::CodePoint::from_u32(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Interpreter, common::encodings::UnicodeNames};

    unsafe extern "C" fn foreign_code(
        _name: *const c_char,
        _length: c_int,
        code: *mut u32,
        _sequences: c_int,
    ) -> c_int {
        // SAFETY: the consumer passes a writable output code point.
        unsafe { code.write(0xD800) };
        1
    }
    static FOREIGN: UnicodeNameCapi = UnicodeNameCapi {
        getname,
        getcode: foreign_code,
    };

    #[test]
    fn native_provider_cache_is_interpreter_local_and_uses_foreign_callbacks() {
        for native in [false, true] {
            Interpreter::without_stdlib(Default::default()).enter(|vm| {
                let modules = vm.sys_module.get_attr("modules", vm).unwrap();
                modules.set_item("unicodedata", vm.ctx.none(), vm).unwrap();
                assert!(vm.unicode_names().is_err());
                let dict = vm.ctx.new_dict();
                // SAFETY: both tables and callback code are immutable/statics.
                let capsule = unsafe {
                    vm.ctx.new_capsule(
                        core::ptr::from_ref(if native { &NAME_API } else { &FOREIGN })
                            .cast_mut()
                            .cast(),
                        Some(CAPSULE_NAME),
                        None,
                    )
                };
                dict.set_item("_ucnhash_CAPI", capsule.into(), vm).unwrap();
                let module = vm.new_module("unicodedata", dict, None);
                modules.set_item("unicodedata", module.into(), vm).unwrap();
                let provider = vm.unicode_names().unwrap();
                assert_eq!(
                    provider
                        .character(b"LATIN CAPITAL LETTER A")
                        .unwrap()
                        .to_u32(),
                    if native { 65 } else { 0xD800 }
                );
                if !native {
                    assert_eq!(provider.character(b"\xff").unwrap().to_u32(), 0xD800);
                }
                modules.del_item("unicodedata", vm).unwrap();
                assert!(core::ptr::eq(provider, vm.unicode_names().unwrap()));
            });
        }
    }

    #[test]
    fn native_getname_buffer_limits() {
        for (code, minimum) in [
            (0x41, 24),
            (0xAC00, 27),
            (0x4E00, 27),
            (0xF0000, 6),
            (0xF900, 33),
            (0x18B00, 36),
            (0x1B170, 22),
            (0x13460, 26),
        ] {
            let mut buffer = [0 as c_char; 256];
            // SAFETY: buffer is writable for all supplied sizes.
            unsafe {
                assert_eq!(getname(code, buffer.as_mut_ptr(), minimum - 1, 1), 0);
                assert_eq!(getname(code, buffer.as_mut_ptr(), minimum, 1), 1);
            }
        }
    }
}
