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

#[pyclass(module = false, name = "map", traverse)]
#[derive(Debug)]
pub struct PyMap {
    mapper: PyObjectRef,
    iterators: Vec<PyIter>,
    #[pytraverse(skip)]
    strict: PyAtomic<bool>,
}

impl PyPayload for PyMap {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.map_type
    }
}

#[derive(FromArgs)]
pub struct PyMapNewArgs {
    #[pyarg(named, optional)]
    strict: OptionalArg<bool>,
}

impl Constructor for PyMap {
    type Args = (PyObjectRef, PosArgs<PyIter>, PyMapNewArgs);

    fn py_new(
        _cls: &Py<PyType>,
        (mapper, iterators, args): Self::Args,
        _vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let iterators = iterators.into_vec();
        let strict = Radium::new(args.strict.unwrap_or(false));
        Ok(Self {
            mapper,
            iterators,
            strict,
        })
    }
}

#[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
impl PyMap {
    #[pymethod]
    fn __length_hint__(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.iterators.iter().try_fold(0, |prev, cur| {
            let cur = cur.as_ref().to_owned().length_hint(0, vm)?;
            let max = core::cmp::max(prev, cur);
            Ok(max)
        })
    }

    #[pymethod]
    fn __reduce__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let cls = zelf.class().to_owned();
        let mut vec = vec![zelf.mapper.clone()];
        vec.extend(zelf.iterators.iter().map(|o| o.clone().into()));
        let tuple_args = vm.ctx.new_tuple(vec);
        Ok(if zelf.strict.load(atomic::Ordering::Acquire) {
            vm.new_tuple((cls, tuple_args, true))
        } else {
            vm.new_tuple((cls, tuple_args))
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

impl SelfIter for PyMap {}

impl IterNext for PyMap {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let mut next_objs = Vec::new();
        for (idx, iterator) in zelf.iterators.iter().enumerate() {
            let item = match iterator.next(vm)? {
                PyIterReturn::Return(obj) => obj,
                PyIterReturn::StopIteration(v) => {
                    if zelf.strict.load(atomic::Ordering::Acquire) {
                        if idx > 0 {
                            let plural = if idx == 1 { " " } else { "s 1-" };
                            return Err(vm.new_value_error(format!(
                                "map() argument {} is shorter than argument{}{}",
                                idx + 1,
                                plural,
                                idx,
                            )));
                        }
                        for (idx, iterator) in zelf.iterators[1..].iter().enumerate() {
                            if let PyIterReturn::Return(_) = iterator.next(vm)? {
                                let plural = if idx == 0 { " " } else { "s 1-" };
                                return Err(vm.new_value_error(format!(
                                    "map() argument {} is longer than argument{}{}",
                                    idx + 2,
                                    plural,
                                    idx + 1,
                                )));
                            }
                        }
                    }
                    return Ok(PyIterReturn::StopIteration(v));
                }
            };
            next_objs.push(item);
        }

        // the mapper itself can raise StopIteration which does stop the map iteration
        PyIterReturn::from_pyresult(zelf.mapper.call(next_objs, vm), vm)
    }
}

pub(crate) fn init(context: &'static Context) {
    PyMap::extend_class(context, context.types.map_type);
}
