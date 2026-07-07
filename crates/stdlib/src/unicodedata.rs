/* Access to the unicode database.
   See also: https://docs.python.org/3/library/unicodedata.html
*/

// spell-checker:ignore nfkc unistr unidata

pub(crate) use unicodedata::module_def;

use rustpython_unicode::{self as unicode_core, NormalizeForm};

use crate::vm::{
    PyObject, PyResult, VirtualMachine, builtins::PyStr, convert::TryFromBorrowedObject,
};

struct NormalizeFormArg(NormalizeForm);

impl<'a> TryFromBorrowedObject<'a> for NormalizeFormArg {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        obj.try_value_with(
            |form: &PyStr| match form.as_bytes() {
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
        builtins::{PyModule, PyStrRef},
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
            module.set_attr(attr, ucd.get_attr(attr, vm)?, vm)?;
        }

        Ok(())
    }

    #[pyattr]
    #[pyclass(name = "UCD")]
    #[derive(Debug, PyPayload)]
    pub(super) struct Ucd {
        inner: unicode_core::Ucd,
    }

    impl Ucd {
        pub(super) const fn new(modern: bool) -> Self {
            Self {
                inner: unicode_core::Ucd::new(modern),
            }
        }

        fn extract_char(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<CodePoint> {
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
        fn category(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<&'static str> {
            self.extract_char(character, vm)
                .map(|c| self.inner.category(c))
        }

        #[pymethod]
        fn lookup(&self, name: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            if let Some(name_str) = name.to_str()
                && let Some(character) = unicode_core::lookup_character(name_str)
            {
                return Ok(character.to_string());
            }
            Err(vm.new_key_error(
                vm.ctx
                    .new_str(format!("undefined character name '{name}'"))
                    .into(),
            ))
        }

        #[pymethod]
        fn name(
            &self,
            character: PyStrRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult {
            if let Some(name) = self
                .extract_char(character, vm)?
                .to_char()
                .and_then(unicode_core::character_name)
            {
                return Ok(vm.ctx.new_str(name).into());
            }
            default.ok_or_else(|| vm.new_value_error("no such name"))
        }

        #[pymethod]
        fn bidirectional(
            &self,
            character: PyStrRef,
            vm: &VirtualMachine,
        ) -> PyResult<&'static str> {
            self.extract_char(character, vm)
                .map(|c| self.inner.bidirectional(c))
        }

        #[pymethod]
        fn east_asian_width(
            &self,
            character: PyStrRef,
            vm: &VirtualMachine,
        ) -> PyResult<&'static str> {
            self.extract_char(character, vm)
                .map(|c| self.inner.east_asian_width(c))
        }

        #[pymethod]
        fn normalize(&self, form: NormalizeFormArg, unistr: PyStrRef) -> Wtf8Buf {
            unicode_core::normalize(form.0, unistr.as_wtf8())
        }

        #[pymethod]
        fn is_normalized(&self, form: NormalizeFormArg, unistr: PyStrRef) -> bool {
            unicode_core::is_normalized(form.0, unistr.as_wtf8())
        }

        #[pymethod]
        fn mirrored(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<i32> {
            self.extract_char(character, vm)
                .map(|c| self.inner.mirrored(c))
        }

        #[pymethod]
        fn combining(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<u8> {
            self.extract_char(character, vm)
                .map(|c| self.inner.combining(c))
        }

        #[pymethod]
        fn decomposition(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            self.extract_char(character, vm)
                .map(|c| self.inner.decomposition(c))
        }

        #[pymethod]
        fn digit(
            &self,
            character: PyStrRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyObjectRef>> {
            let ch = self.extract_char(character, vm)?;
            self.inner
                .digit(ch)
                .map(|value| vm.ctx.new_int(value).into())
                .or_else(|| default.present())
                .map(Option::Some)
                .ok_or_else(|| vm.new_value_error("not a digit"))
        }

        #[pymethod]
        fn decimal(
            &self,
            character: PyStrRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyObjectRef>> {
            let ch = self.extract_char(character, vm)?;
            self.inner
                .decimal(ch)
                .map(|value| vm.ctx.new_int(value).into())
                .or_else(|| default.present())
                .map(Option::Some)
                .ok_or_else(|| vm.new_value_error("not a decimal"))
        }

        #[pymethod]
        fn numeric(
            &self,
            character: PyStrRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyObjectRef>> {
            let ch = self.extract_char(character, vm)?;
            self.inner
                .numeric(ch)
                .map(|value| vm.ctx.new_float(value).into())
                .or_else(|| default.present())
                .map(Option::Some)
                .ok_or_else(|| vm.new_value_error("not a numeric character"))
        }

        #[pygetset]
        fn unidata_version(&self) -> String {
            self.inner.unidata_version()
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
