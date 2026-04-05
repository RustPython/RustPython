/* Access to the unicode database.
   See also: https://docs.python.org/3/library/unicodedata.html
*/

// spell-checker:ignore nfkc unistr unidata

pub(crate) use unicodedata::module_def;

#[pymodule]
mod unicodedata {
    use crate::vm::{
        Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyModule, PyStrRef},
        function::OptionalArg,
    };

    use itertools::Itertools;
    use rustpython_common::wtf8::{CodePoint, Wtf8Buf};
    use rustpython_unicode::{NormalizeForm, UNICODE_VERSION, UnicodeVersion, data};

    fn parse_normalize_form(form: PyStrRef, vm: &VirtualMachine) -> PyResult<NormalizeForm> {
        form.to_str()
            .ok_or_else(|| vm.new_value_error("invalid normalization form"))?
            .parse()
            .map_err(|()| vm.new_value_error("invalid normalization form"))
    }

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);

        // Add UCD methods as module-level functions
        let ucd: PyObjectRef = PyUcd::new(data::Ucd::default()).into_ref(&vm.ctx).into();

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
    pub(super) struct PyUcd(data::Ucd);

    impl PyUcd {
        pub const fn new(ucd: data::Ucd) -> Self {
            Self(ucd)
        }

        fn extract_char(character: PyStrRef, vm: &VirtualMachine) -> PyResult<CodePoint> {
            character
                .as_wtf8()
                .code_points()
                .exactly_one()
                .map_err(|_| vm.new_type_error("argument must be an unicode character, not str"))
        }
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl PyUcd {
        #[pymethod]
        fn category(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            Ok(self
                .0
                .category(Self::extract_char(character, vm)?.to_u32())
                .to_owned())
        }

        #[pymethod]
        fn lookup(&self, name: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            if let Some(name_str) = name.to_str()
                && let Some(character) = self.0.lookup(name_str)
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
            if let Some(name) = self.0.name(Self::extract_char(character, vm)?.to_u32()) {
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
                .0
                .bidirectional(Self::extract_char(character, vm)?.to_u32()))
        }

        /// NOTE: This function uses 9.0.0 database instead of 3.2.0
        #[pymethod]
        fn east_asian_width(
            &self,
            character: PyStrRef,
            vm: &VirtualMachine,
        ) -> PyResult<&'static str> {
            Ok(self
                .0
                .east_asian_width(Self::extract_char(character, vm)?.to_u32()))
        }

        #[pymethod]
        fn normalize(
            &self,
            form: PyStrRef,
            unistr: PyStrRef,
            vm: &VirtualMachine,
        ) -> PyResult<Wtf8Buf> {
            Ok(self
                .0
                .normalize(parse_normalize_form(form, vm)?, unistr.as_wtf8()))
        }

        #[pymethod]
        fn is_normalized(
            &self,
            form: PyStrRef,
            unistr: PyStrRef,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
            Ok(self
                .0
                .is_normalized(parse_normalize_form(form, vm)?, unistr.as_wtf8()))
        }

        #[pymethod]
        fn mirrored(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<i32> {
            Ok(self.0.mirrored(Self::extract_char(character, vm)?.to_u32()) as i32)
        }

        #[pymethod]
        fn combining(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<u8> {
            Ok(self
                .0
                .combining(Self::extract_char(character, vm)?.to_u32()))
        }

        #[pymethod]
        fn decomposition(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            Ok(self
                .0
                .decomposition(Self::extract_char(character, vm)?.to_u32()))
        }

        #[pymethod]
        fn digit(
            &self,
            character: PyStrRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult {
            if let Some(value) = self.0.digit(Self::extract_char(character, vm)?.to_u32()) {
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
            if let Some(value) = self.0.decimal(Self::extract_char(character, vm)?.to_u32()) {
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
            if let Some(value) = self.0.numeric(Self::extract_char(character, vm)?.to_u32()) {
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
            self.0.unicode_version().to_string()
        }
    }

    #[pyattr]
    fn ucd_3_2_0(vm: &VirtualMachine) -> PyRef<PyUcd> {
        PyUcd::new(data::Ucd::new(UnicodeVersion {
            major: 3,
            minor: 2,
            micro: 0,
        }))
        .into_ref(&vm.ctx)
    }

    #[pyattr]
    fn unidata_version(_vm: &VirtualMachine) -> String {
        UNICODE_VERSION.to_string()
    }
}
