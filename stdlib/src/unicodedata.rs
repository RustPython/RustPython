/* Access to the unicode database.
   See also: https://docs.python.org/3/library/unicodedata.html
*/
use crate::vm::{PyObjectRef, PyPayload, VirtualMachine};

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let module = unicodedata::make_module(vm);

    let ucd: PyObjectRef = unicodedata::Ucd::new(unic_ucd_age::UNICODE_VERSION)
        .into_ref(vm)
        .into();

    for attr in [
        "category",
        "lookup",
        "name",
        "bidirectional",
        "east_asian_width",
        "normalize",
    ]
    .into_iter()
    {
        crate::vm::extend_module!(vm, &module, {
            attr => ucd.get_attr(attr, vm).unwrap(),
        });
    }

    module
}

#[pymodule]
mod unicodedata {
    use crate::vm::{
        builtins::PyStrRef, function::OptionalArg, PyObjectRef, PyPayload, PyRef, PyResult,
        VirtualMachine,
    };
    use itertools::Itertools;
    use ucd::{Codepoint, EastAsianWidth};
    use unic_char_property::EnumeratedCharProperty;
    use unic_normal::StrNormalForm;
    use unic_ucd_age::{Age, UnicodeVersion, UNICODE_VERSION};
    use unic_ucd_bidi::BidiClass;
    use unic_ucd_category::GeneralCategory;

    #[pyattr]
    #[pyclass(name = "UCD")]
    #[derive(Debug, PyPayload)]
    pub(super) struct Ucd {
        unic_version: UnicodeVersion,
    }

    impl Ucd {
        pub fn new(unic_version: UnicodeVersion) -> Self {
            Self { unic_version }
        }

        fn check_age(&self, c: char) -> bool {
            Age::of(c).map_or(false, |age| age.actual() <= self.unic_version)
        }

        fn extract_char(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<Option<char>> {
            let c = character.as_str().chars().exactly_one().map_err(|_| {
                vm.new_type_error("argument must be an unicode character, not str".to_owned())
            })?;

            if self.check_age(c) {
                Ok(Some(c))
            } else {
                Ok(None)
            }
        }
    }

    #[pyclass]
    impl Ucd {
        #[pymethod]
        fn category(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            Ok(self
                .extract_char(character, vm)?
                .map_or(GeneralCategory::Unassigned, GeneralCategory::of)
                .abbr_name()
                .to_owned())
        }

        #[pymethod]
        fn lookup(&self, name: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            if let Some(character) = unicode_names2::character(name.as_str()) {
                if self.check_age(character) {
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
                    if let Some(name) = unicode_names2::name(c) {
                        return Ok(vm.ctx.new_str(name.to_string()).into());
                    }
                }
            }
            default.ok_or_else(|| vm.new_value_error("character name not found!".to_owned()))
        }

        #[pymethod]
        fn bidirectional(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            let bidi = match self.extract_char(character, vm)? {
                Some(c) => BidiClass::of(c).abbr_name(),
                None => "",
            };
            Ok(bidi.to_owned())
        }

        /// NOTE: This function uses 9.0.0 database instead of 3.2.0
        #[pymethod]
        fn east_asian_width(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
            Ok(self
                .extract_char(character, vm)?
                .map_or(EastAsianWidth::Neutral, |c| c.east_asian_width())
                .abbr_name()
                .to_owned())
        }

        #[pymethod]
        fn normalize(
            &self,
            form: PyStrRef,
            unistr: PyStrRef,
            vm: &VirtualMachine,
        ) -> PyResult<String> {
            let text = unistr.as_str();
            let normalized_text = match form.as_str() {
                "NFC" => text.nfc().collect::<String>(),
                "NFKC" => text.nfkc().collect::<String>(),
                "NFD" => text.nfd().collect::<String>(),
                "NFKD" => text.nfkd().collect::<String>(),
                _ => return Err(vm.new_value_error("invalid normalization form".to_owned())),
            };

            Ok(normalized_text)
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
                EastAsianWidth::Narrow => "Na",
                EastAsianWidth::Wide => "W",
                EastAsianWidth::Neutral => "N",
                EastAsianWidth::Ambiguous => "A",
                EastAsianWidth::FullWidth => "F",
                EastAsianWidth::HalfWidth => "H",
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
        .into_ref(vm)
    }

    #[pyattr]
    fn unidata_version(_vm: &VirtualMachine) -> String {
        UNICODE_VERSION.to_string()
    }
}
