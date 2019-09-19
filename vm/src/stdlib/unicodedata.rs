/* Access to the unicode database.
   See also: https://docs.python.org/3/library/unicodedata.html
*/

use crate::function::OptionalArg;
use crate::obj::objstr::PyStringRef;
use crate::pyobject::{PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

use unic::char::property::EnumeratedCharProperty;
use unic::ucd::category::GeneralCategory;
use unic::ucd::Name;
use unicode_names2;

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let unidata_version = unic::UNICODE_VERSION.to_string();

    py_module!(vm, "unicodedata", {
        "bidirectional" => ctx.new_rustfunc(bidirectional),
        "category" => ctx.new_rustfunc(category),
        "name" => ctx.new_rustfunc(name),
        "lookup" => ctx.new_rustfunc(lookup),
        "normalize" => ctx.new_rustfunc(normalize),
        "unidata_version" => ctx.new_str(unidata_version),
    })
}

fn category(character: PyStringRef, vm: &VirtualMachine) -> PyResult {
    let my_char = extract_char(character, vm)?;
    let category = GeneralCategory::of(my_char);
    Ok(vm.new_str(category.abbr_name().to_string()))
}

fn lookup(name: PyStringRef, vm: &VirtualMachine) -> PyResult {
    // TODO: we might want to use unic_ucd instead of unicode_names2 for this too, if possible:
    if let Some(character) = unicode_names2::character(name.as_str()) {
        Ok(vm.new_str(character.to_string()))
    } else {
        Err(vm.new_key_error(vm.new_str(format!("undefined character name '{}'", name))))
    }
}

fn name(
    character: PyStringRef,
    default: OptionalArg<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult {
    let my_char = extract_char(character, vm)?;

    if let Some(name) = Name::of(my_char) {
        Ok(vm.new_str(name.to_string()))
    } else {
        match default {
            OptionalArg::Present(obj) => Ok(obj),
            OptionalArg::Missing => {
                Err(vm.new_value_error("character name not found!".to_string()))
            }
        }
    }
}

fn bidirectional(character: PyStringRef, vm: &VirtualMachine) -> PyResult {
    use unic::bidi::BidiClass;
    let my_char = extract_char(character, vm)?;
    let cls = BidiClass::of(my_char);
    Ok(vm.new_str(cls.abbr_name().to_string()))
}

fn normalize(form: PyStringRef, unistr: PyStringRef, vm: &VirtualMachine) -> PyResult {
    use unic::normal::StrNormalForm;
    let text = unistr.as_str();
    let normalized_text = match form.as_str() {
        "NFC" => text.nfc().collect::<String>(),
        "NFKC" => text.nfkc().collect::<String>(),
        "NFD" => text.nfd().collect::<String>(),
        "NFKD" => text.nfkd().collect::<String>(),
        _ => {
            return Err(vm.new_value_error("unistr must be one of NFC, NFD".to_string()));
        }
    };

    Ok(vm.new_str(normalized_text))
}

fn extract_char(character: PyStringRef, vm: &VirtualMachine) -> PyResult<char> {
    if character.as_str().len() != 1 {
        return Err(vm.new_type_error("argument must be an unicode character, not str".to_string()));
    }

    let my_char: char = character.as_str().chars().next().unwrap();
    Ok(my_char)
}
