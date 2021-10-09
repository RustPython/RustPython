use super::PyTypeRef;
use crate::{
    function::{ArgIntoBool, IntoPyObject, OptionalArg, PosArgs},
    protocol::{PyIter, PyIterReturn},
    slots::{IteratorIterable, SlotConstructor, SlotIterator},
    PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject, TypeProtocol,
    VirtualMachine,
};
use rustpython_common::atomic::{self, PyAtomic, Radium};

#[pyclass(module = false, name = "zip")]
#[derive(Debug)]
pub struct PyZip {
    iterators: Vec<PyIter>,
    strict: PyAtomic<bool>,
}

impl PyValue for PyZip {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.zip_type
    }
}

#[derive(FromArgs)]
pub struct PyZipNewArgs {
    #[pyarg(named, optional)]
    strict: OptionalArg<bool>,
}

impl SlotConstructor for PyZip {
    type Args = (PosArgs<PyIter>, PyZipNewArgs);

    fn py_new(cls: PyTypeRef, (iterators, args): Self::Args, vm: &VirtualMachine) -> PyResult {
        let iterators = iterators.into_vec();
        let strict = Radium::new(args.strict.unwrap_or(false));
        PyZip { iterators, strict }.into_pyresult_with_type(vm, cls)
    }
}

#[pyimpl(with(SlotIterator, SlotConstructor), flags(BASETYPE))]
impl PyZip {
    #[pymethod(magic)]
    fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let cls = zelf.clone_class().into_pyobject(vm);
        let iterators = zelf
            .iterators
            .iter()
            .map(|obj| obj.clone().into())
            .collect::<Vec<_>>();
        let tuple_iter = vm.ctx.new_tuple(iterators);
        Ok(if zelf.strict.load(atomic::Ordering::Acquire) {
            vm.ctx
                .new_tuple(vec![cls, tuple_iter, vm.ctx.new_bool(true)])
        } else {
            vm.ctx.new_tuple(vec![cls, tuple_iter])
        })
    }

    #[pymethod(magic)]
    fn setstate(zelf: PyRef<Self>, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if let Ok(obj) = ArgIntoBool::try_from_object(vm, state) {
            zelf.strict.store(obj.to_bool(), atomic::Ordering::Release);
        }
        Ok(())
    }
}

impl IteratorIterable for PyZip {}
impl SlotIterator for PyZip {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        if zelf.iterators.is_empty() {
            return Ok(PyIterReturn::StopIteration(None));
        }
        let mut next_objs = Vec::new();
        for (idx, iterator) in zelf.iterators.iter().enumerate() {
            let item = match iterator.next(vm)? {
                PyIterReturn::Return(obj) => obj,
                PyIterReturn::StopIteration(v) => {
                    if zelf.strict.load(atomic::Ordering::Acquire) {
                        if idx > 0 {
                            let plural = if idx == 1 { " " } else { "s 1-" };
                            return Err(vm.new_value_error(format!(
                                "zip() argument {} is shorter than argument{}{}",
                                idx + 1,
                                plural,
                                idx
                            )));
                        }
                        for (idx, iterator) in zelf.iterators[1..].iter().enumerate() {
                            if let PyIterReturn::Return(_obj) = iterator.next(vm)? {
                                let plural = if idx == 0 { " " } else { "s 1-" };
                                return Err(vm.new_value_error(format!(
                                    "zip() argument {} is longer than argument{}{}",
                                    idx + 2,
                                    plural,
                                    idx + 1
                                )));
                            }
                        }
                    }
                    return Ok(PyIterReturn::StopIteration(v));
                }
            };
            next_objs.push(item);
        }
        Ok(PyIterReturn::Return(vm.ctx.new_tuple(next_objs)))
    }
}

pub fn init(context: &PyContext) {
    PyZip::extend_class(context, &context.types.zip_type);
}
