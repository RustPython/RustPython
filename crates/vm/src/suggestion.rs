//! This module provides functionality to suggest similar names for attributes or variables.
//! This is used during tracebacks.

use crate::{
    AsObject, Py, PyObject, PyObjectRef, VirtualMachine,
    builtins::{PyStr, PyStrRef},
    exceptions::types::PyBaseException,
    sliceable::SliceableSequenceOp,
};
use core::iter::ExactSizeIterator;
use rustpython_common::str::levenshtein::{MOVE_COST, levenshtein_distance};

const MAX_CANDIDATE_ITEMS: usize = 750;

pub fn calculate_suggestions<'a>(
    dir_iter: impl ExactSizeIterator<Item = &'a PyObjectRef>,
    name: &PyObject,
) -> Option<PyStrRef> {
    if dir_iter.len() >= MAX_CANDIDATE_ITEMS {
        return None;
    }

    let mut suggestion: Option<&Py<PyStr>> = None;
    let mut suggestion_distance = usize::MAX;
    let name = name.downcast_ref::<PyStr>()?;

    for item in dir_iter {
        let item_name = item.downcast_ref::<PyStr>()?;
        if name.as_bytes() == item_name.as_bytes() {
            continue;
        }
        // No more than 1/3 of the characters should need changed
        let max_distance = usize::min(
            (name.len() + item_name.len() + 3) * MOVE_COST / 6,
            suggestion_distance - 1,
        );
        let current_distance =
            levenshtein_distance(name.as_bytes(), item_name.as_bytes(), max_distance);
        if current_distance > max_distance {
            continue;
        }
        if suggestion.is_none() || current_distance < suggestion_distance {
            suggestion = Some(item_name);
            suggestion_distance = current_distance;
        }
    }
    suggestion.map(|r| r.to_owned())
}

pub fn offer_suggestions(exc: &Py<PyBaseException>, vm: &VirtualMachine) -> Option<PyStrRef> {
    if exc
        .class()
        .fast_issubclass(vm.ctx.exceptions.attribute_error)
    {
        let name = exc.as_object().get_attr("name", vm).ok()?;
        if vm.is_none(&name) {
            return None;
        }
        let obj = exc.as_object().get_attr("obj", vm).ok()?;
        if vm.is_none(&obj) {
            return None;
        }

        calculate_suggestions(vm.dir(Some(obj)).ok()?.borrow_vec().iter(), &name)
    } else if exc.class().fast_issubclass(vm.ctx.exceptions.name_error) {
        let name = exc.as_object().get_attr("name", vm).ok()?;
        if vm.is_none(&name) {
            return None;
        }
        let tb = exc.__traceback__()?;
        let tb = tb.iter().last().unwrap_or(tb);

        let varnames = tb.frame.code.clone().co_varnames(vm);
        if let Some(suggestions) = calculate_suggestions(varnames.iter(), &name) {
            return Some(suggestions);
        };

        let globals: Vec<_> = tb.frame.globals.as_object().try_to_value(vm).ok()?;
        if let Some(suggestions) = calculate_suggestions(globals.iter(), &name) {
            return Some(suggestions);
        };

        let builtins: Vec<_> = tb.frame.builtins.try_to_value(vm).ok()?;
        calculate_suggestions(builtins.iter(), &name)
    } else if exc.class().fast_issubclass(vm.ctx.exceptions.import_error) {
        let mod_name = exc.as_object().get_attr("name", vm).ok()?;
        let wrong_name = exc.as_object().get_attr("name_from", vm).ok()?;
        let mod_name_str = mod_name.downcast_ref::<PyStr>()?;

        // Look up the module in sys.modules
        let sys_modules = vm.sys_module.get_attr("modules", vm).ok()?;
        let module = sys_modules.get_item(mod_name_str.as_str(), vm).ok()?;

        calculate_suggestions(vm.dir(Some(module)).ok()?.borrow_vec().iter(), &wrong_name)
    } else {
        None
    }
}
