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
    use super::NormalizeForm::*;
    use crate::vm::{
        Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyModule, PyStrRef},
        function::OptionalArg,
    };
    use itertools::Itertools;
    use rustpython_common::wtf8::{CodePoint, Wtf8Buf};
    use ucd::{Codepoint, DecompositionType, EastAsianWidth, Number, NumericType};
    use unic_char_property::EnumeratedCharProperty;
    use unic_normal::StrNormalForm;
    use unic_ucd_age::{Age, UNICODE_VERSION, UnicodeVersion};
    use unic_ucd_bidi::BidiClass;
    use unic_ucd_category::GeneralCategory;
    use unicode_bidi_mirroring::is_mirroring;

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

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
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
            if let Some(name_str) = name.to_str()
                && let Some(character) = unicode_names2::character(name_str)
                && self.check_age(character.into())
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
            let c = self.extract_char(character, vm)?;

            if let Some(c) = c
                && self.check_age(c)
                && let Some(name) = c.to_char().and_then(unicode_names2::name)
            {
                return Ok(vm.ctx.new_str(name.to_string()).into());
            }
            default.ok_or_else(|| vm.new_value_error("no such name"))
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
        fn is_normalized(&self, form: super::NormalizeForm, unistr: PyStrRef) -> PyResult<bool> {
            let text = unistr.as_wtf8();
            let normalized: Wtf8Buf = match form {
                Nfc => text.map_utf8(|s| s.nfc()).collect(),
                Nfkc => text.map_utf8(|s| s.nfkc()).collect(),
                Nfd => text.map_utf8(|s| s.nfd()).collect(),
                Nfkd => text.map_utf8(|s| s.nfkd()).collect(),
            };
            Ok(text == &*normalized)
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

        #[pymethod]
        fn combining(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<i32> {
            Ok(self
                .extract_char(character, vm)?
                .and_then(|c| c.to_char())
                .map_or(0, |ch| ch.canonical_combining_class() as i32))
        }

        #[pymethod]
        fn decomposition(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            let ch = match self.extract_char(character, vm)?.and_then(|c| c.to_char()) {
                Some(ch) => ch,
                None => return Ok(String::new()),
            };
            let chars: Vec<char> = ch.decomposition_map().collect();
            // If decomposition maps to just the character itself, there's no decomposition
            if chars.len() == 1 && chars[0] == ch {
                return Ok(String::new());
            }
            let hex_parts = chars.iter().map(|c| format!("{:04X}", *c as u32)).join(" ");
            let tag = match ch.decomposition_type() {
                Some(DecompositionType::Canonical) | None => return Ok(hex_parts),
                Some(dt) => decomposition_type_tag(dt),
            };
            Ok(format!("<{tag}> {hex_parts}"))
        }

        #[pymethod]
        fn digit(
            &self,
            character: PyStrRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let ch = self.extract_char(character, vm)?.and_then(|c| c.to_char());
            if let Some(ch) = ch
                && matches!(
                    ch.numeric_type(),
                    Some(NumericType::Decimal) | Some(NumericType::Digit)
                )
                && let Some(Number::Integer(n)) = ch.numeric_value()
            {
                return Ok(vm.ctx.new_int(n).into());
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
            let ch = self.extract_char(character, vm)?.and_then(|c| c.to_char());
            if let Some(ch) = ch
                && ch.numeric_type() == Some(NumericType::Decimal)
                && let Some(Number::Integer(n)) = ch.numeric_value()
            {
                return Ok(vm.ctx.new_int(n).into());
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
            let ch = self.extract_char(character, vm)?.and_then(|c| c.to_char());
            if let Some(ch) = ch {
                match ch.numeric_value() {
                    Some(Number::Integer(n)) => {
                        return Ok(vm.ctx.new_float(n as f64).into());
                    }
                    Some(Number::Rational(num, den)) => {
                        return Ok(vm.ctx.new_float(num as f64 / den as f64).into());
                    }
                    None => {}
                }
            }
            default.ok_or_else(|| vm.new_value_error("not a numeric character"))
        }

        #[pygetset]
        fn unidata_version(&self) -> String {
            self.unic_version.to_string()
        }
    }

    fn decomposition_type_tag(dt: DecompositionType) -> &'static str {
        match dt {
            DecompositionType::Canonical => "canonical",
            DecompositionType::Compat => "compat",
            DecompositionType::Circle => "circle",
            DecompositionType::Final => "final",
            DecompositionType::Font => "font",
            DecompositionType::Fraction => "fraction",
            DecompositionType::Initial => "initial",
            DecompositionType::Isolated => "isolated",
            DecompositionType::Medial => "medial",
            DecompositionType::Narrow => "narrow",
            DecompositionType::Nobreak => "noBreak",
            DecompositionType::Small => "small",
            DecompositionType::Square => "square",
            DecompositionType::Sub => "sub",
            DecompositionType::Super => "super",
            DecompositionType::Vertical => "vertical",
            DecompositionType::Wide => "wide",
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
