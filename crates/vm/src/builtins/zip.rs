use super::PyType;
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
    builtins::PyTupleRef,
    class::PyClassImpl,
    function::{ArgIntoBool, OptionalArg, PosArgs},
    protocol::{PyIter, PyIterReturn},
    types::{Constructor, IterNext, Iterable, SelfIter},
};
use rustpython_common::atomic::{self, PyAtomic, Radium};

#[pyclass(module = false, name = "zip", traverse)]
#[derive(Debug)]
pub struct PyZip {
    iterators: Vec<PyIter>,
    #[pytraverse(skip)]
    strict: PyAtomic<bool>,
}

impl PyPayload for PyZip {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.zip_type
    }
}

#[derive(Clone, Copy, FromArgs)]
pub struct PyZipNewArgs {
    #[pyarg(named, optional)]
    strict: OptionalArg<bool>,
}

impl Constructor for PyZip {
    type Args = (PosArgs<PyIter>, PyZipNewArgs);

    fn py_new(
        _cls: &Py<PyType>,
        (iterators, args): Self::Args,
        _vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let iterators = iterators.into_vec();
        let strict = Radium::new(args.strict.unwrap_or(false));
        Ok(Self { iterators, strict })
    }
}

#[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
impl PyZip {
    #[pymethod]
    fn __reduce__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let cls = zelf.class().to_owned();
        let iterators = zelf
            .iterators
            .iter()
            .map(|obj| obj.clone().into())
            .collect::<Vec<_>>();
        let tuple_iter = vm.ctx.new_tuple(iterators);
        Ok(if zelf.strict.load(atomic::Ordering::Acquire) {
            vm.new_tuple((cls, tuple_iter, true))
        } else {
            vm.new_tuple((cls, tuple_iter))
        })
    }

    #[pymethod]
    fn __setstate__(zelf: PyRef<Self>, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if let Ok(obj) = ArgIntoBool::try_from_object(vm, state) {
            zelf.strict.store(obj.into(), atomic::Ordering::Release);
        }
        Ok(())
    }
}

impl SelfIter for PyZip {}
impl IterNext for PyZip {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
        Ok(PyIterReturn::Return(vm.ctx.new_tuple(next_objs).into()))
    }
}

pub fn init(ctx: &Context) {
    PyZip::extend_class(ctx, ctx.types.zip_type);
}
