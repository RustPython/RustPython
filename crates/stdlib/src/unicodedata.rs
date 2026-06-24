/* Access to the unicode database.
   See also: https://docs.python.org/3/library/unicodedata.html
*/

// spell-checker:ignore codep decomp DECOMP nfkc unistr unidata

use core::{cmp::Ordering, hint::cold_path};

pub(crate) use unicodedata::module_def;

use icu_properties::props::{
    BidiClass, CanonicalCombiningClass, EastAsianWidth, GeneralCategory, NumericType,
};

use crate::vm::{
    PyObject, PyResult, VirtualMachine, builtins::PyStr, convert::TryFromBorrowedObject,
};

include!(concat!(env!("OUT_DIR"), "/generated/unicode_3_2.rs"));
include!(concat!(env!("OUT_DIR"), "/generated/unicode_latest.rs"));
include!(concat!(env!("OUT_DIR"), "/generated/unicode_num_type.rs"));
include!(concat!(
    env!("OUT_DIR"),
    "/generated/unicode_numeric_value.rs"
));

#[derive(Clone, Copy)]
#[repr(u8)]
enum DecompositionType {
    #[allow(unused)]
    Canonical,
    Compat,
    Circle,
    Final,
    Font,
    Fraction,
    Initial,
    Isolated,
    Medial,
    Narrow,
    Nobreak,
    Small,
    Square,
    Sub,
    Super,
    Vertical,
    Wide,
}

impl DecompositionType {
    const fn type_tag(self) -> &'static str {
        match self {
            Self::Canonical => "canonical",
            Self::Compat => "compat",
            Self::Circle => "circle",
            Self::Final => "final",
            Self::Font => "font",
            Self::Fraction => "fraction",
            Self::Initial => "initial",
            Self::Isolated => "isolated",
            Self::Medial => "medial",
            Self::Narrow => "narrow",
            Self::Nobreak => "noBreak",
            Self::Small => "small",
            Self::Square => "square",
            Self::Sub => "sub",
            Self::Super => "super",
            Self::Vertical => "vertical",
            Self::Wide => "wide",
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum NormalizeForm {
    Nfc,
    Nfkc,
    Nfd,
    Nfkd,
}

fn lookup_property<T: Copy>(table: &[(u32, u32, T)], ch: char) -> Option<T> {
    let ch = ch as u32;
    table
        .binary_search_by(|&(start, end, _)| {
            if ch > end {
                Ordering::Less
            } else if ch < start {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        })
        .ok()
        .map(|i| table[i].2)
}

fn lookup_numeric_val(ch: char, modern: bool) -> Option<f64> {
    if modern {
        lookup_property(NUMERIC_VALUES, ch)
    } else {
        cold_path();
        lookup_property(NUMERIC_VALUES_DIFF, ch).or_else(|| {
            NUMERIC_VAL_EXISTS_32
                .binary_search_by(|&(start, end)| {
                    let ch = ch as u32;
                    if ch > end {
                        Ordering::Less
                    } else if ch < start {
                        Ordering::Greater
                    } else {
                        Ordering::Equal
                    }
                })
                .ok()
                .and_then(|_| lookup_property(NUMERIC_VALUES, ch))
        })
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
    use core::{cmp::Ordering, fmt::Write, hint::cold_path};

    use super::{
        BIDI_CLASS, BIDI_MIRRORED, COMBINING_CLASS, DECOMP_COMPAT, DECOMP_RANGE, DECOMP_UPDATES,
        EAST_ASIAN_WIDTH, GENERAL_CATEGORY, NUMERIC_TYPE_DIFF, NormalizeForm, lookup_numeric_val,
        lookup_property,
    };
    use crate::vm::{
        Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyModule, PyStrRef},
        function::OptionalArg,
    };

    use icu_normalizer::{
        ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed,
        properties::{CanonicalDecomposition, Decomposed},
    };
    use icu_properties::props::{
        BidiClass, BidiMirrored, BinaryProperty, CanonicalCombiningClass, EastAsianWidth,
        EnumeratedProperty, GeneralCategory, NamedEnumeratedProperty, NumericType,
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
        modern: bool,
    }

    impl Ucd {
        pub(super) const fn new(modern: bool) -> Self {
            Self { modern }
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
            self.extract_char(character, vm).map(|c| {
                let Some(c) = c.to_char() else {
                    return GeneralCategory::Surrogate.short_name();
                };
                if self.modern {
                    Some(GeneralCategory::for_char(c))
                } else {
                    cold_path();
                    lookup_property(GENERAL_CATEGORY, c)
                }
                .unwrap_or(GeneralCategory::Unassigned)
                .short_name()
            })
        }

        // TODO: Names needs to account for Unicode 3.2.0 and 16.0.0
        #[pymethod]
        fn lookup(&self, name: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            if let Some(name_str) = name.to_str()
                && let Some(character) = unicode_names2::character(name_str)
            {
                return Ok(character.to_string());
            }
            Err(vm.new_key_error(
                vm.ctx
                    .new_str(format!("undefined character name '{name}'"))
                    .into(),
            ))
        }

        // TODO: Names needs to account for Unicode 3.2.0 and 16.0.0
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
                .and_then(unicode_names2::name)
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
            self.extract_char(character, vm).map(|c| {
                c.to_char()
                    .and_then(|c| {
                        if self.modern {
                            Some(BidiClass::for_char(c))
                        } else {
                            cold_path();
                            lookup_property(BIDI_CLASS, c)
                        }
                    })
                    .unwrap_or(BidiClass::LeftToRight)
                    .short_name()
            })
        }

        #[pymethod]
        fn east_asian_width(
            &self,
            character: PyStrRef,
            vm: &VirtualMachine,
        ) -> PyResult<&'static str> {
            self.extract_char(character, vm).map(|c| {
                c.to_char()
                    .and_then(|c| {
                        if self.modern {
                            Some(EastAsianWidth::for_char(c))
                        } else {
                            cold_path();
                            // CPython overrides characters in the PUA for 3.2.0.
                            // Basic Multilingual Plane:
                            // https://en.wikipedia.org/wiki/Plane_(Unicode)#Basic_Multilingual_Plane
                            // https://en.wikipedia.org/wiki/Private_Use_Areas
                            // https://www.unicode.org/reports/tr11/tr11-10.html
                            // https://www.unicode.org/reports/tr11/
                            //
                            // Currently, this implementation is incomplete because I can't figure
                            // out what CPython is doing.
                            lookup_property(EAST_ASIAN_WIDTH, c)
                        }
                    })
                    .unwrap_or(EastAsianWidth::Neutral)
                    .short_name()
            })
        }

        #[pymethod]
        fn normalize(&self, form: super::NormalizeForm, unistr: PyStrRef) -> Wtf8Buf {
            let text = unistr.as_wtf8();
            match form {
                NormalizeForm::Nfc => {
                    let normalizer = ComposingNormalizerBorrowed::new_nfc();
                    text.map_utf8(|s| normalizer.normalize_iter(s.chars()))
                        .collect()
                }
                NormalizeForm::Nfkc => {
                    let normalizer = ComposingNormalizerBorrowed::new_nfkc();
                    text.map_utf8(|s| normalizer.normalize_iter(s.chars()))
                        .collect()
                }
                NormalizeForm::Nfd => {
                    let normalizer = DecomposingNormalizerBorrowed::new_nfd();
                    text.map_utf8(|s| normalizer.normalize_iter(s.chars()))
                        .collect()
                }
                NormalizeForm::Nfkd => {
                    let normalizer = DecomposingNormalizerBorrowed::new_nfkd();
                    text.map_utf8(|s| normalizer.normalize_iter(s.chars()))
                        .collect()
                }
            }
        }

        #[pymethod]
        fn is_normalized(&self, form: super::NormalizeForm, unistr: PyStrRef) -> bool {
            let text = unistr.as_wtf8();
            let normalized: Wtf8Buf = match form {
                NormalizeForm::Nfc => {
                    let normalizer = ComposingNormalizerBorrowed::new_nfc();
                    text.map_utf8(|s| normalizer.normalize_iter(s.chars()))
                        .collect()
                }
                NormalizeForm::Nfkc => {
                    let normalizer = ComposingNormalizerBorrowed::new_nfkc();
                    text.map_utf8(|s| normalizer.normalize_iter(s.chars()))
                        .collect()
                }
                NormalizeForm::Nfd => {
                    let normalizer = DecomposingNormalizerBorrowed::new_nfd();
                    text.map_utf8(|s| normalizer.normalize_iter(s.chars()))
                        .collect()
                }
                NormalizeForm::Nfkd => {
                    let normalizer = DecomposingNormalizerBorrowed::new_nfkd();
                    text.map_utf8(|s| normalizer.normalize_iter(s.chars()))
                        .collect()
                }
            };
            text == &*normalized
        }

        #[pymethod]
        fn mirrored(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<i32> {
            self.extract_char(character, vm).map(|c| {
                c.to_char().map_or(0, |c| {
                    (if self.modern {
                        BidiMirrored::for_char(c)
                    } else {
                        cold_path();
                        let c = c as u32;
                        BIDI_MIRRORED
                            .binary_search_by(|&(start, end)| {
                                if c > end {
                                    Ordering::Less
                                } else if c < start {
                                    Ordering::Greater
                                } else {
                                    Ordering::Equal
                                }
                            })
                            .is_ok()
                    }) as i32
                })
            })
        }

        #[pymethod]
        fn combining(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<u8> {
            self.extract_char(character, vm).map(|c| {
                c.to_char()
                    .and_then(|c| {
                        if self.modern {
                            Some(CanonicalCombiningClass::for_char(c))
                        } else {
                            cold_path();
                            lookup_property(COMBINING_CLASS, c)
                        }
                    })
                    .unwrap_or(CanonicalCombiningClass::NotReordered)
                    .to_icu4c_value()
            })
        }

        #[pymethod]
        fn decomposition(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            let Some(ch) = self.extract_char(character, vm).map(CodePoint::to_char)? else {
                return Ok(String::new());
            };

            // Decomposition is remarkable stable according to the normalization file,
            // so the updates slice is very small - only about four char pairs. Linearly searching
            // it is very fast. The file lists the original, incorrect decomp and the fixed char.
            // For 3.2.0, we use the original decomp for compatibility while ignoring the update.
            //
            // Finally, we don't have to do anything for the latest UCD as it's already updated.
            if self.modern
                && let Some((_, original)) = DECOMP_UPDATES
                    .iter()
                    .find(|&&(codep, _original)| codep == ch as u32)
            {
                Ok(format!("{original:04X}"))
            } else if let Ok(i) =
                DECOMP_COMPAT.binary_search_by_key(&(ch as u32), |&(codep, _, _)| codep)
            {
                // Compatibility decomposition
                // `icu4x` doesn't expose a non-recursive, compatibility decomposer so we
                // have to do it manually for now.
                let tag = DECOMP_COMPAT[i].1.type_tag();
                let end = DECOMP_COMPAT[i].2;
                let start = i
                    .checked_sub(1)
                    .map(|i| DECOMP_COMPAT[i].2)
                    .unwrap_or_default();

                let decomp = &DECOMP_RANGE[start..end];
                let cap = decomp.len() * 10 + decomp.len() + tag.len() + 1;
                let mut out = String::with_capacity(cap);

                write!(out, "<{tag}>").unwrap();
                for ch in decomp {
                    write!(out, " {ch:04X}").unwrap();
                }

                Ok(out)
            } else {
                // Canonical decomposition
                let decomposed = CanonicalDecomposition::new().decompose(ch);
                match decomposed {
                    Decomposed::Default => Ok(String::new()),
                    Decomposed::Singleton(ch) => Ok(format!("{:04X}", ch as u32)),
                    Decomposed::Expansion(l, r) => Ok(format!("{:04X} {:04X}", l as u32, r as u32)),
                }
            }
        }

        fn numeric_type_matches(&self, ch: CodePoint, expected: &[NumericType]) -> Option<char> {
            let ch = ch.to_char()?;

            let actual = if self.modern {
                NumericType::for_char(ch)
            } else {
                cold_path();
                lookup_property(NUMERIC_TYPE_DIFF, ch).unwrap_or_else(|| NumericType::for_char(ch))
            };

            expected.contains(&actual).then_some(ch)
        }

        #[pymethod]
        fn digit(
            &self,
            character: PyStrRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyObjectRef>> {
            let ch = self.extract_char(character, vm)?;
            let expected = [NumericType::Decimal, NumericType::Digit];
            self.numeric_type_matches(ch, &expected)
                .and_then(|ch| {
                    let value = lookup_numeric_val(ch, true)?;
                    (value.trunc() == value).then(|| vm.ctx.new_int(value as u64).into())
                })
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
            let expected = [NumericType::Decimal];
            self.numeric_type_matches(ch, &expected)
                .and_then(|ch| {
                    let value = lookup_numeric_val(ch, self.modern)?;
                    (value.trunc() == value).then(|| vm.ctx.new_int(value as u64).into())
                })
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
            let expected = &NumericType::ALL_VALUES[1..];
            self.numeric_type_matches(ch, expected)
                .and_then(|ch| {
                    lookup_numeric_val(ch, self.modern).map(|value| vm.ctx.new_float(value).into())
                })
                .or_else(|| default.present())
                .map(Option::Some)
                .ok_or_else(|| vm.new_value_error("not a numeric character"))
        }

        #[pygetset]
        fn unidata_version(&self) -> String {
            if self.modern {
                format!(
                    "{}.{}.{}",
                    char::UNICODE_VERSION.0,
                    char::UNICODE_VERSION.1,
                    char::UNICODE_VERSION.2
                )
            } else {
                "3.2.0".into()
            }
        }
    }

    #[pyattr]
    fn ucd_3_2_0(vm: &VirtualMachine) -> PyRef<Ucd> {
        Ucd::new(false).into_ref(&vm.ctx)
    }

    #[pyattr]
    fn unidata_version(_vm: &VirtualMachine) -> String {
        format!(
            "{}.{}.{}",
            char::UNICODE_VERSION.0,
            char::UNICODE_VERSION.1,
            char::UNICODE_VERSION.2
        )
    }
}
