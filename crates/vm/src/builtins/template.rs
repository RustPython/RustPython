use super::{PyStr, PyTupleRef, PyType};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    atomic_func,
    class::PyClassImpl,
    function::{FuncArgs, PyComparisonValue},
    protocol::{PyIterReturn, PySequenceMethods},
    types::{
        AsSequence, Comparable, Constructor, IterNext, Iterable, PyComparisonOp, Representable,
        SelfIter,
    },
};
use std::sync::LazyLock;

use super::interpolation::PyInterpolation;

/// Template object for t-strings (PEP 750).
///
/// Represents a template string with interpolated expressions.
#[pyclass(module = "string.templatelib", name = "Template")]
#[derive(Debug, Clone)]
pub struct PyTemplate {
    pub strings: PyTupleRef,
    pub interpolations: PyTupleRef,
}

impl PyPayload for PyTemplate {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.template_type
    }
}

impl PyTemplate {
    pub fn new(strings: PyTupleRef, interpolations: PyTupleRef) -> Self {
        Self {
            strings,
            interpolations,
        }
    }
}

impl Constructor for PyTemplate {
    type Args = FuncArgs;

    fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        if !args.kwargs.is_empty() {
            return Err(vm.new_type_error("Template.__new__ only accepts *args arguments"));
        }

        let mut strings: Vec<PyObjectRef> = Vec::new();
        let mut interpolations: Vec<PyObjectRef> = Vec::new();
        let mut last_was_str = false;

        for item in args.args.iter() {
            if let Ok(s) = item.clone().downcast::<PyStr>() {
                if last_was_str {
                    // Concatenate adjacent strings
                    if let Some(last) = strings.last_mut() {
                        let last_str = last.downcast_ref::<PyStr>().unwrap();
                        let concatenated = format!("{}{}", last_str.as_str(), s.as_str());
                        *last = vm.ctx.new_str(concatenated).into();
                    }
                } else {
                    strings.push(s.into());
                }
                last_was_str = true;
            } else if item.class().is(vm.ctx.types.interpolation_type) {
                if !last_was_str {
                    // Add empty string before interpolation
                    strings.push(vm.ctx.empty_str.to_owned().into());
                }
                interpolations.push(item.clone());
                last_was_str = false;
            } else {
                return Err(vm.new_type_error(format!(
                    "Template.__new__ *args need to be of type 'str' or 'Interpolation', got {}",
                    item.class().name()
                )));
            }
        }

        if !last_was_str {
            // Add trailing empty string
            strings.push(vm.ctx.empty_str.to_owned().into());
        }

        Ok(PyTemplate {
            strings: vm.ctx.new_tuple(strings),
            interpolations: vm.ctx.new_tuple(interpolations),
        })
    }
}

#[pyclass(with(Constructor, Comparable, Iterable, Representable, AsSequence))]
impl PyTemplate {
    #[pygetset]
    fn strings(&self) -> PyTupleRef {
        self.strings.clone()
    }

    #[pygetset]
    fn interpolations(&self) -> PyTupleRef {
        self.interpolations.clone()
    }

    #[pygetset]
    fn values(&self, vm: &VirtualMachine) -> PyTupleRef {
        let values: Vec<PyObjectRef> = self
            .interpolations
            .iter()
            .map(|interp| {
                interp
                    .downcast_ref::<PyInterpolation>()
                    .map(|i| i.value.clone())
                    .unwrap_or_else(|| interp.clone())
            })
            .collect();
        vm.ctx.new_tuple(values)
    }

    fn concat(&self, other: &PyObject, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let other = other.downcast_ref::<PyTemplate>().ok_or_else(|| {
            vm.new_type_error(format!(
                "can only concatenate Template (not '{}') to Template",
                other.class().name()
            ))
        })?;

        // Concatenate the two templates
        let mut new_strings: Vec<PyObjectRef> = Vec::new();
        let mut new_interps: Vec<PyObjectRef> = Vec::new();

        // Add all strings from self except the last one
        let self_strings_len = self.strings.len();
        for i in 0..self_strings_len.saturating_sub(1) {
            new_strings.push(self.strings.get(i).unwrap().clone());
        }

        // Add all interpolations from self
        for interp in self.interpolations.iter() {
            new_interps.push(interp.clone());
        }

        // Concatenate last string of self with first string of other
        let last_self = self
            .strings
            .get(self_strings_len.saturating_sub(1))
            .and_then(|s| s.downcast_ref::<PyStr>().map(|s| s.as_str().to_owned()))
            .unwrap_or_default();
        let first_other = other
            .strings
            .first()
            .and_then(|s| s.downcast_ref::<PyStr>().map(|s| s.as_str().to_owned()))
            .unwrap_or_default();
        let concatenated = format!("{}{}", last_self, first_other);
        new_strings.push(vm.ctx.new_str(concatenated).into());

        // Add remaining strings from other (skip first)
        for i in 1..other.strings.len() {
            new_strings.push(other.strings.get(i).unwrap().clone());
        }

        // Add all interpolations from other
        for interp in other.interpolations.iter() {
            new_interps.push(interp.clone());
        }

        let template = PyTemplate {
            strings: vm.ctx.new_tuple(new_strings),
            interpolations: vm.ctx.new_tuple(new_interps),
        };

        Ok(template.into_ref(&vm.ctx))
    }

    fn __add__(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        self.concat(&other, vm)
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        // Import string.templatelib._template_unpickle
        // We need to import string first, then get templatelib from it,
        // because import("string.templatelib", 0) with empty from_list returns the top-level module
        let string_mod = vm.import("string.templatelib", 0)?;
        let templatelib = string_mod.get_attr("templatelib", vm)?;
        let unpickle_func = templatelib.get_attr("_template_unpickle", vm)?;

        // Return (func, (strings, interpolations))
        let args = vm.ctx.new_tuple(vec![
            self.strings.clone().into(),
            self.interpolations.clone().into(),
        ]);
        Ok(vm.ctx.new_tuple(vec![unpickle_func, args.into()]))
    }
}

impl AsSequence for PyTemplate {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
            concat: atomic_func!(|seq, other, vm| {
                let zelf = PyTemplate::sequence_downcast(seq);
                zelf.concat(other, vm).map(|t| t.into())
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl Comparable for PyTemplate {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| {
            let other = class_or_notimplemented!(Self, other);

            let eq = vm.bool_eq(zelf.strings.as_object(), other.strings.as_object())?
                && vm.bool_eq(
                    zelf.interpolations.as_object(),
                    other.interpolations.as_object(),
                )?;

            Ok(eq.into())
        })
    }
}

impl Iterable for PyTemplate {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(PyTemplateIter::new(zelf).into_pyobject(vm))
    }
}

impl Representable for PyTemplate {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let mut parts = Vec::new();

        let strings_len = zelf.strings.len();
        let interps_len = zelf.interpolations.len();

        for i in 0..strings_len.max(interps_len * 2 + 1) {
            if i % 2 == 0 {
                // String position
                let idx = i / 2;
                if idx < strings_len {
                    let s = zelf.strings.get(idx).unwrap();
                    parts.push(s.repr(vm)?.as_str().to_owned());
                }
            } else {
                // Interpolation position
                let idx = i / 2;
                if idx < interps_len {
                    let interp = zelf.interpolations.get(idx).unwrap();
                    parts.push(interp.repr(vm)?.as_str().to_owned());
                }
            }
        }

        Ok(format!("Template({})", parts.join(", ")))
    }
}

/// Iterator for Template objects
#[pyclass(module = "string.templatelib", name = "TemplateIter")]
#[derive(Debug)]
pub struct PyTemplateIter {
    template: PyRef<PyTemplate>,
    index: std::sync::atomic::AtomicUsize,
    from_strings: std::sync::atomic::AtomicBool,
}

impl PyPayload for PyTemplateIter {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.template_iter_type
    }
}

impl PyTemplateIter {
    fn new(template: PyRef<PyTemplate>) -> Self {
        Self {
            template,
            index: std::sync::atomic::AtomicUsize::new(0),
            from_strings: std::sync::atomic::AtomicBool::new(true),
        }
    }
}

#[pyclass(with(IterNext, Iterable))]
impl PyTemplateIter {}

impl SelfIter for PyTemplateIter {}

impl IterNext for PyTemplateIter {
    fn next(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        use std::sync::atomic::Ordering;

        loop {
            let from_strings = zelf.from_strings.load(Ordering::SeqCst);
            let index = zelf.index.load(Ordering::SeqCst);

            if from_strings {
                if index < zelf.template.strings.len() {
                    let item = zelf.template.strings.get(index).unwrap();
                    zelf.from_strings.store(false, Ordering::SeqCst);

                    // Skip empty strings
                    if let Some(s) = item.downcast_ref::<PyStr>()
                        && s.as_str().is_empty()
                    {
                        continue;
                    }
                    return Ok(PyIterReturn::Return(item.clone()));
                } else {
                    return Ok(PyIterReturn::StopIteration(None));
                }
            } else if index < zelf.template.interpolations.len() {
                let item = zelf.template.interpolations.get(index).unwrap();
                zelf.index.fetch_add(1, Ordering::SeqCst);
                zelf.from_strings.store(true, Ordering::SeqCst);
                return Ok(PyIterReturn::Return(item.clone()));
            } else {
                return Ok(PyIterReturn::StopIteration(None));
            }
        }
    }
}

pub fn init(context: &Context) {
    PyTemplate::extend_class(context, context.types.template_type);
    PyTemplateIter::extend_class(context, context.types.template_iter_type);
}
