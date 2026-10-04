/* Access to the unicode database.
   See also: https://docs.python.org/3/library/unicodedata.html
*/

// spell-checker:ignore nfkc unistr unidata

pub(crate) use unicodedata::module_def;

use rustpython_unicode::{self as unicode_core, NormalizeForm};

use crate::vm::{
    Py, PyObject, PyResult, VirtualMachine, builtins::PyStr, convert::TryFromBorrowedObject,
};

struct NormalizeFormArg(NormalizeForm);

impl<'a> TryFromBorrowedObject<'a> for NormalizeFormArg {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        obj.try_value_with(
            |form: &Py<PyStr>| match form.as_bytes() {
                b"NFC" => Ok(Self(NormalizeForm::Nfc)),
                b"NFKC" => Ok(Self(NormalizeForm::Nfkc)),
                b"NFD" => Ok(Self(NormalizeForm::Nfd)),
                b"NFKD" => Ok(Self(NormalizeForm::Nfkd)),
                _ => Err(vm.new_value_error("invalid normalization form")),
            },
            vm,
        )
    }
}

#[pymodule]
mod unicodedata {
    use super::{NormalizeFormArg, unicode_core};
    use crate::vm::{
        Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyModule, PyStr, PyStrRef},
        function::OptionalArg,
    };
    use itertools::Itertools;
    use rustpython_common::wtf8::{CodePoint, Wtf8Buf};

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);

        // Add UCD methods as module-level functions
        let ucd: PyObjectRef = Ucd::new(true).into_ref(&vm.ctx).into();

        for attr in [
            "category",
            "lookup",
            "name",
            "bidirectional",
            "combining",
            "decimal",
            "decomposition",
            "digit",
            "east_asian_width",
            "is_normalized",
            "mirrored",
            "normalize",
            "numeric",
        ] {
            let func = ucd.get_attr(attr, vm)?;
            func.set_attr("__module__", vm.ctx.new_str("unicodedata"), vm)?;
            module.set_attr(attr, func, vm)?;
        }

        Ok(())
    }

    #[pyattr]
    #[pyclass(name = "UCD")]
    #[derive(Debug, PyPayload)]
    pub(super) struct Ucd {
        inner: unicode_core::Ucd,
        /// Owns the bytes `unidata_version` points at.
        #[expect(dead_code, reason = "keeps the version string alive")]
        version_owner: alloc::ffi::CString,
        #[pymember]
        unidata_version: crate::vm::builtins::descriptor::CStrMember,
    }

    impl Ucd {
        pub(super) fn new(modern: bool) -> Self {
            let text = if modern {
                unicode_core::unicode_version()
            } else {
                "3.2.0".to_owned()
            };
            let version_owner = alloc::ffi::CString::new(text).unwrap_or_default();
            let unidata_version =
                crate::vm::builtins::descriptor::CStrMember::new(version_owner.as_ptr());
            Self {
                inner: unicode_core::Ucd::new(modern),
                version_owner,
                unidata_version,
            }
        }

        fn extract_char(&self, character: &Py<PyStr>, vm: &VirtualMachine) -> PyResult<CodePoint> {
            character
                .as_wtf8()
                .code_points()
                .exactly_one()
                .map_err(|_| vm.new_type_error("argument must be an unicode character, not str"))
        }
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl Ucd {
        #[pymethod]
        fn category(zelf: &Py<Self>, chr: PyStrRef, vm: &VirtualMachine) -> PyResult<&'static str> {
            zelf.extract_char(&chr, vm).map(|c| zelf.inner.category(c))
        }

        #[pymethod]
        fn lookup(zelf: &Py<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            if let Some(name_str) = name.to_str()
                && let Some(found) = zelf.inner.lookup(name_str)
            {
                return Ok(match found {
                    unicode_core::LookupResult::Character(ch) => ch.to_string(),
                    unicode_core::LookupResult::Sequence(seq) => seq.to_string(),
                });
            }
            Err(vm.new_key_error(
                vm.ctx
                    .new_str(format!("undefined character name '{name}'"))
                    .into(),
            ))
        }

        #[pymethod]
        fn name(
            zelf: &Py<Self>,
            chr: PyStrRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult {
            if let Some(name) = zelf.extract_char(&chr, vm)?.to_char().and_then(|ch| {
                zelf.inner
                    .membership(ch)
                    .then(|| unicode_core::character_name(ch))
                    .flatten()
            }) {
                return Ok(vm.ctx.new_str(name).into());
            }
            default.ok_or_else(|| vm.new_value_error("no such name"))
        }

        #[pymethod]
        fn bidirectional(
            zelf: &Py<Self>,
            chr: PyStrRef,
            vm: &VirtualMachine,
        ) -> PyResult<&'static str> {
            zelf.extract_char(&chr, vm)
                .map(|c| zelf.inner.bidirectional(c))
        }

        #[pymethod]
        fn east_asian_width(
            zelf: &Py<Self>,
            chr: PyStrRef,
            vm: &VirtualMachine,
        ) -> PyResult<&'static str> {
            zelf.extract_char(&chr, vm)
                .map(|c| zelf.inner.east_asian_width(c))
        }

        #[pymethod]
        fn normalize(_zelf: &Py<Self>, form: NormalizeFormArg, unistr: PyStrRef) -> Wtf8Buf {
            unicode_core::normalize(form.0, unistr.as_wtf8())
        }

        #[pymethod]
        fn is_normalized(_zelf: &Py<Self>, form: NormalizeFormArg, unistr: PyStrRef) -> bool {
            unicode_core::is_normalized(form.0, unistr.as_wtf8())
        }

        #[pymethod]
        fn mirrored(zelf: &Py<Self>, chr: PyStrRef, vm: &VirtualMachine) -> PyResult<i32> {
            zelf.extract_char(&chr, vm).map(|c| zelf.inner.mirrored(c))
        }

        #[pymethod]
        fn combining(zelf: &Py<Self>, chr: PyStrRef, vm: &VirtualMachine) -> PyResult<u8> {
            zelf.extract_char(&chr, vm).map(|c| zelf.inner.combining(c))
        }

        #[pymethod]
        fn decomposition(zelf: &Py<Self>, chr: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            zelf.extract_char(&chr, vm)
                .map(|c| zelf.inner.decomposition(c))
        }

        #[pymethod]
        fn digit(
            zelf: &Py<Self>,
            chr: PyStrRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyObjectRef>> {
            let ch = zelf.extract_char(&chr, vm)?;
            zelf.inner
                .digit(ch)
                .map(|value| vm.ctx.new_int(value).into())
                .or_else(|| default.present())
                .map(Option::Some)
                .ok_or_else(|| vm.new_value_error("not a digit"))
        }

        #[pymethod]
        fn decimal(
            zelf: &Py<Self>,
            chr: PyStrRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyObjectRef>> {
            let ch = zelf.extract_char(&chr, vm)?;
            zelf.inner
                .decimal(ch)
                .map(|value| vm.ctx.new_int(value).into())
                .or_else(|| default.present())
                .map(Option::Some)
                .ok_or_else(|| vm.new_value_error("not a decimal"))
        }

        #[pymethod]
        fn numeric(
            zelf: &Py<Self>,
            chr: PyStrRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyObjectRef>> {
            let ch = zelf.extract_char(&chr, vm)?;
            zelf.inner
                .numeric(ch)
                .map(|value| vm.ctx.new_float(value).into())
                .or_else(|| default.present())
                .map(Option::Some)
                .ok_or_else(|| vm.new_value_error("not a numeric character"))
        }
    }

    #[pyattr]
    fn ucd_3_2_0(vm: &VirtualMachine) -> PyRef<Ucd> {
        Ucd::new(false).into_ref(&vm.ctx)
    }

    #[pyattr]
    fn unidata_version(_vm: &VirtualMachine) -> String {
        unicode_core::unicode_version()
    }
}
