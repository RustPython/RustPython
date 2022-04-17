use crate::{
    builtins::{PyStr, PyStrRef},
    exceptions::types::PyBaseExceptionRef,
    sliceable::SliceableSequenceOp,
    AsPyObject, PyObjectRef, PyObjectView, TypeProtocol, VirtualMachine,
};
use rustpython_common::str::levenshtein::{levenshtein_distance, MOVE_COST};
use std::iter::ExactSizeIterator;

const MAX_CANDIDATE_ITEMS: usize = 750;

fn calculate_suggestions<'a>(
    dir_iter: impl ExactSizeIterator<Item = &'a PyObjectRef>,
    name: &PyObjectRef,
) -> Option<PyStrRef> {
    if dir_iter.len() >= MAX_CANDIDATE_ITEMS {
        return None;
    }

    let mut suggestion: Option<&PyObjectView<PyStr>> = None;
    let mut suggestion_distance = usize::MAX;
    let name = name.downcast_ref::<PyStr>()?;

    for item in dir_iter {
        let item_name = item.downcast_ref::<PyStr>()?;
        if name.as_str() == item_name.as_str() {
            continue;
        }
        // No more than 1/3 of the characters should need changed
        let max_distance = usize::min(
            (name.len() + item_name.len() + 3) * MOVE_COST / 6,
            suggestion_distance - 1,
        );
        let current_distance =
            levenshtein_distance(name.as_str(), item_name.as_str(), max_distance);
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

pub fn offer_suggestions(exc: &PyBaseExceptionRef, vm: &VirtualMachine) -> Option<PyStrRef> {
    if exc.class().is(&vm.ctx.exceptions.attribute_error) {
        let name = exc.as_object().to_owned().get_attr("name", vm).unwrap();
        let obj = exc.as_object().to_owned().get_attr("obj", vm).unwrap();

        calculate_suggestions(vm.dir(Some(obj)).ok()?.borrow_vec().iter(), &name)
    } else if exc.class().is(&vm.ctx.exceptions.name_error) {
        let name = exc.as_object().to_owned().get_attr("name", vm).unwrap();
        let mut tb = exc.traceback().unwrap();
        while let Some(traceback) = tb.next.clone() {
            tb = traceback;
        }

        let varnames = tb.frame.code.clone().co_varnames(vm);
        if let Some(suggestions) = calculate_suggestions(varnames.as_slice().iter(), &name) {
            return Some(suggestions);
        };

        let globals: Vec<_> = tb.frame.globals.as_object().try_to_value(vm).ok()?;
        if let Some(suggestions) = calculate_suggestions(globals.as_slice().iter(), &name) {
            return Some(suggestions);
        };

        let builtins: Vec<_> = tb.frame.builtins.as_object().try_to_value(vm).ok()?;
        calculate_suggestions(builtins.as_slice().iter(), &name)
    } else {
        None
    }
}
