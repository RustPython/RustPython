/* Access to the unicode database.
   See also: https://docs.python.org/3/library/unicodedata.html
*/

use crate::vm::{
    builtins::PyStrRef, extend_module, function::OptionalArg, py_module, PyClassImpl, PyObject,
    PyObjectRef, PyResult, PyValue, VirtualMachine,
};
use itertools::Itertools;
use unic_char_property::EnumeratedCharProperty;
use unic_normal::StrNormalForm;
use unic_ucd_age::{Age, UnicodeVersion, UNICODE_VERSION};
use unic_ucd_bidi::BidiClass;
use unic_ucd_category::GeneralCategory;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let ucd_class = PyUCD::make_class(ctx);

    let ucd = PyObject::new(PyUCD::default(), ucd_class.clone(), None);

    let ucd_3_2_0 = PyObject::new(
        PyUCD {
            unic_version: UnicodeVersion {
                major: 3,
                minor: 2,
                micro: 0,
            },
        },
        ucd_class.clone(),
        None,
    );

    let module = py_module!(vm, "unicodedata", {
        "UCD" => ucd_class.into_object(),
        "ucd_3_2_0" => ucd_3_2_0,
        // we do unidata_version here because the getter tries to do PyUCD::class() before
        // the module is in the VM
        "unidata_version" => ctx.new_utf8_str(PyUCD::default().unic_version.to_string()),
    });

    for attr in ["category", "lookup", "name", "bidirectional", "normalize"]
        .iter()
        .copied()
    {
        extend_module!(vm, &module, {
            attr => vm.get_attribute(ucd.clone(), attr).unwrap(),
        });
    }

    module
}

#[pyclass(module = "unicodedata", name = "UCD")]
#[derive(Debug, PyValue)]
struct PyUCD {
    unic_version: UnicodeVersion,
}

impl Default for PyUCD {
    #[inline(always)]
    fn default() -> Self {
        PyUCD {
            unic_version: UNICODE_VERSION,
        }
    }
}

#[pyimpl]
impl PyUCD {
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
        Err(vm.new_lookup_error(format!("undefined character name '{}'", name)))
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
                    return Ok(vm.ctx.new_utf8_str(name.to_string()));
                }
            }
        }
        match default {
            OptionalArg::Present(obj) => Ok(obj),
            OptionalArg::Missing => Err(vm.new_value_error("character name not found!".to_owned())),
        }
    }

    #[pymethod]
    fn bidirectional(&self, character: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
        let bidi = match self.extract_char(character, vm)? {
            Some(c) => BidiClass::of(c).abbr_name(),
            None => "",
        };
        Ok(bidi.to_owned())
    }

    #[pymethod]
    fn normalize(&self, form: PyStrRef, unistr: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
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

    #[pyproperty]
    fn unidata_version(&self) -> String {
        self.unic_version.to_string()
    }
}
