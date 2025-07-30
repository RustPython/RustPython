/* Access to the unicode database.
   See also: https://docs.python.org/3/library/unicodedata.html
*/

// spell-checker:ignore nfkc unistr unidata

use crate::vm::{
    PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine, builtins::PyModule,
    builtins::PyStr, convert::TryFromBorrowedObject,
};

pub fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let module = unicodedata::make_module(vm);

    let ucd: PyObjectRef = unicodedata::Ucd::new(unic_ucd_age::UNICODE_VERSION)
        .into_ref(&vm.ctx)
        .into();

    for attr in [
        "category",
        "lookup",
        "name",
        "bidirectional",
        "east_asian_width",
        "normalize",
        "mirrored",
    ] {
        crate::vm::extend_module!(vm, &module, {
            attr => ucd.get_attr(attr, vm).unwrap(),
        });
    }

    module
}

enum NormalizeForm {
    Nfc,
    Nfkc,
    Nfd,
    Nfkd,
}

impl<'a> TryFromBorrowedObject<'a> for NormalizeForm {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        obj.try_value_with(
            |form: &PyStr| {
                Ok(match form.as_str() {
                    "NFC" => Self::Nfc,
                    "NFKC" => Self::Nfkc,
                    "NFD" => Self::Nfd,
                    "NFKD" => Self::Nfkd,
                    _ => return Err(vm.new_value_error("invalid normalization form")),
                })
            },
            vm,
        )
    }
}

#[pymodule]
mod unicodedata {
    use crate::vm::{
        PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine, builtins::PyStrRef,
        function::OptionalArg,
    };
    use itertools::Itertools;
    use rustpython_common::wtf8::{CodePoint, Wtf8Buf};
    use ucd::{Codepoint, EastAsianWidth};
    use unic_char_property::EnumeratedCharProperty;
    use unic_normal::StrNormalForm;
    use unic_ucd_age::{Age, UNICODE_VERSION, UnicodeVersion};
    use unic_ucd_bidi::BidiClass;
    use unic_ucd_category::GeneralCategory;
    use unicode_bidi_mirroring::is_mirroring;

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
            c.to_char()
                .is_none_or(|c| Age::of(c).is_some_and(|age| age.actual() <= self.unic_version))
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

    #[pyclass]
    impl Ucd {
        #[pymethod]
        fn category(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            Ok(self
                .extract_char(character, vm)?
                .map_or(GeneralCategory::Unassigned, |c| {
                    c.to_char()
                        .map_or(GeneralCategory::Surrogate, GeneralCategory::of)
                })
                .abbr_name()
                .to_owned())
        }

        #[pymethod]
        fn lookup(&self, name: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            if let Some(character) = unicode_names2::character(name.as_str()) {
                if self.check_age(character.into()) {
                    return Ok(character.to_string());
                }
            }
            Err(vm.new_lookup_error(format!("undefined character name '{name}'")))
        }

        #[pymethod]
        fn name(
            &self,
            character: PyStrRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let c = self.extract_char(character, vm)?;

            if let Some(c) = c {
                if self.check_age(c) {
                    if let Some(name) = c.to_char().and_then(unicode_names2::name) {
                        return Ok(vm.ctx.new_str(name.to_string()).into());
                    }
                }
            }
            default.ok_or_else(|| vm.new_value_error("character name not found!"))
        }

        #[pymethod]
        fn bidirectional(
            &self,
            character: PyStrRef,
            vm: &VirtualMachine,
        ) -> PyResult<&'static str> {
            let bidi = match self.extract_char(character, vm)? {
                Some(c) => c
                    .to_char()
                    .map_or(BidiClass::LeftToRight, BidiClass::of)
                    .abbr_name(),
                None => "",
            };
            Ok(bidi)
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
                .and_then(|c| c.to_char())
                .map_or(EastAsianWidth::Neutral, |c| c.east_asian_width())
                .abbr_name())
        }

        #[pymethod]
        fn normalize(&self, form: super::NormalizeForm, unistr: PyStrRef) -> PyResult<Wtf8Buf> {
            use super::NormalizeForm::*;
            let text = unistr.as_wtf8();
            let normalized_text = match form {
                Nfc => text.map_utf8(|s| s.nfc()).collect(),
                Nfkc => text.map_utf8(|s| s.nfkc()).collect(),
                Nfd => text.map_utf8(|s| s.nfd()).collect(),
                Nfkd => text.map_utf8(|s| s.nfkd()).collect(),
            };
            Ok(normalized_text)
        }

        #[pymethod]
        fn mirrored(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<i32> {
            match self.extract_char(character, vm)? {
                Some(c) => {
                    if let Some(ch) = c.to_char() {
                        // Check if the character is mirrored in bidirectional text using Unicode standard
                        Ok(if is_mirroring(ch) { 1 } else { 0 })
                    } else {
                        Ok(0)
                    }
                }
                None => Ok(0),
            }
        }

        #[pygetset]
        fn unidata_version(&self) -> String {
            self.unic_version.to_string()
        }
    }

    trait EastAsianWidthAbbrName {
        fn abbr_name(&self) -> &'static str;
    }

    impl EastAsianWidthAbbrName for EastAsianWidth {
        fn abbr_name(&self) -> &'static str {
            match self {
                Self::Narrow => "Na",
                Self::Wide => "W",
                Self::Neutral => "N",
                Self::Ambiguous => "A",
                Self::FullWidth => "F",
                Self::HalfWidth => "H",
            }
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
