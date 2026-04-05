/* Access to the unicode database.
   See also: https://docs.python.org/3/library/unicodedata.html
*/

// spell-checker:ignore nfkc unistr unidata

pub(crate) use unicodedata::module_def;

use crate::vm::{
    PyObject, PyResult, VirtualMachine, builtins::PyStr, convert::TryFromBorrowedObject,
};

enum NormalizeForm {
    Nfc,
    Nfkc,
    Nfd,
    Nfkd,
}

impl From<NormalizeForm> for rustpython_unicode::NormalizeForm {
    fn from(value: NormalizeForm) -> Self {
        match value {
            NormalizeForm::Nfc => Self::Nfc,
            NormalizeForm::Nfkc => Self::Nfkc,
            NormalizeForm::Nfd => Self::Nfd,
            NormalizeForm::Nfkd => Self::Nfkd,
        }
    }
}

impl<'a> TryFromBorrowedObject<'a> for NormalizeForm {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        obj.try_value_with(
            |form: &PyStr| match form.as_bytes() {
                b"NFC" => Ok(Self::Nfc),
                b"NFKC" => Ok(Self::Nfkc),
                b"NFD" => Ok(Self::Nfd),
                b"NFKD" => Ok(Self::Nfkd),
                _ => Err(vm.new_value_error("invalid normalization form")),
            },
            vm,
        )
    }
}

#[pymodule]
mod unicodedata {
    use crate::vm::{
        Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyModule, PyStrRef},
        function::OptionalArg,
    };

    use itertools::Itertools;
    use rustpython_common::wtf8::{CodePoint, Wtf8Buf};
    use rustpython_unicode::{UNICODE_VERSION, UnicodeVersion, data, normalize};

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);

        // Add UCD methods as module-level functions
        let ucd: PyObjectRef = Ucd::new(UNICODE_VERSION).into_ref(&vm.ctx).into();

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
        unic_version: UnicodeVersion,
    }

    impl Ucd {
        pub const fn new(unic_version: UnicodeVersion) -> Self {
            Self { unic_version }
        }

        fn check_age(&self, c: CodePoint) -> bool {
            data::is_assigned_in_version(c.to_u32(), self.unic_version)
        }

        fn extract_char(
            &self,
            character: PyStrRef,
            vm: &VirtualMachine,
        ) -> PyResult<Option<CodePoint>> {
            let c = character
                .as_wtf8()
                .code_points()
                .exactly_one()
                .map_err(|_| vm.new_type_error("argument must be an unicode character, not str"))?;

            Ok(self.check_age(c).then_some(c))
        }
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl Ucd {
        #[pymethod]
        fn category(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            Ok(self
                .extract_char(character, vm)?
                .map_or("Cn", |c| data::category(c.to_u32()))
                .to_owned())
        }

        #[pymethod]
        fn lookup(&self, name: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            if let Some(name_str) = name.to_str()
                && let Some(character) = data::lookup(name_str)
                && self.check_age(CodePoint::from_u32(character).expect("valid Unicode code point"))
            {
                return Ok(char::from_u32(character)
                    .expect("unicode_names2 only returns Unicode scalar values")
                    .to_string());
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
            let c = self.extract_char(character, vm)?;

            if let Some(c) = c
                && self.check_age(c)
                && let Some(name) = data::name(c.to_u32())
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
            Ok(self
                .extract_char(character, vm)?
                .map_or("", |c| data::bidirectional(c.to_u32())))
        }

        /// NOTE: This function uses 9.0.0 database instead of 3.2.0
        #[pymethod]
        fn east_asian_width(
            &self,
            character: PyStrRef,
            vm: &VirtualMachine,
        ) -> PyResult<&'static str> {
            Ok(self
                .extract_char(character, vm)?
                .map_or("N", |c| data::east_asian_width(c.to_u32())))
        }

        #[pymethod]
        fn normalize(&self, form: super::NormalizeForm, unistr: PyStrRef) -> PyResult<Wtf8Buf> {
            Ok(normalize::normalize(form.into(), unistr.as_wtf8()))
        }

        #[pymethod]
        fn is_normalized(&self, form: super::NormalizeForm, unistr: PyStrRef) -> PyResult<bool> {
            Ok(normalize::is_normalized(form.into(), unistr.as_wtf8()))
        }

        #[pymethod]
        fn mirrored(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<i32> {
            Ok(self
                .extract_char(character, vm)?
                .is_some_and(|c| data::mirrored(c.to_u32())) as i32)
        }

        #[pymethod]
        fn combining(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<u8> {
            Ok(self
                .extract_char(character, vm)?
                .map_or(0, |c| data::combining(c.to_u32())))
        }

        #[pymethod]
        fn decomposition(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            Ok(self
                .extract_char(character, vm)?
                .map_or_else(String::new, |c| data::decomposition(c.to_u32())))
        }

        #[pymethod]
        fn digit(
            &self,
            character: PyStrRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult {
            if let Some(value) = self
                .extract_char(character, vm)?
                .and_then(|c| data::digit(c.to_u32()))
            {
                return Ok(vm.ctx.new_int(value).into());
            }
            default.ok_or_else(|| vm.new_value_error("not a digit"))
        }

        #[pymethod]
        fn decimal(
            &self,
            character: PyStrRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult {
            if let Some(value) = self
                .extract_char(character, vm)?
                .and_then(|c| data::decimal(c.to_u32()))
            {
                return Ok(vm.ctx.new_int(value).into());
            }
            default.ok_or_else(|| vm.new_value_error("not a decimal"))
        }

        #[pymethod]
        fn numeric(
            &self,
            character: PyStrRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult {
            if let Some(value) = self
                .extract_char(character, vm)?
                .and_then(|c| data::numeric(c.to_u32()))
            {
                let value = match value {
                    data::NumericValue::Integer(n) => n as f64,
                    data::NumericValue::Rational(num, den) => num as f64 / den as f64,
                };
                return Ok(vm.ctx.new_float(value).into());
            }
            default.ok_or_else(|| vm.new_value_error("not a numeric character"))
        }

        #[pygetset]
        fn unidata_version(&self) -> String {
            self.unic_version.to_string()
        }
    }

    #[pyattr]
    fn ucd_3_2_0(vm: &VirtualMachine) -> PyRef<Ucd> {
        Ucd {
            unic_version: UnicodeVersion {
                major: 3,
                minor: 2,
                micro: 0,
            },
        }
        .into_ref(&vm.ctx)
    }

    #[pyattr]
    fn unidata_version(_vm: &VirtualMachine) -> String {
        UNICODE_VERSION.to_string()
    }
}
