use super::{PyType, PyTypeRef};
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
    builtins::PyTupleRef,
    class::PyClassImpl,
    function::{ArgIntoBool, FuncArgs, PosArgs},
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
    #[pyarg(positional)]
    function: PyObjectRef,
    #[pyarg(positional)]
    iterable: PyIter,
    #[pyarg(flatten)]
    iterables: PosArgs<PyIter, crate::function::NameIterables>,
    #[pyarg(named, default)]
    strict: bool,
}

#[derive(FromArgs)]
struct MapCallArgs {
    #[pyarg(positional)]
    function: PyObjectRef,
    #[pyarg(flatten)]
    iterables: PosArgs<PyIter, crate::function::NameIterables>,
    #[pyarg(named, default)]
    strict: bool,
}

impl Constructor for PyMap {
    type Args = PyMapNewArgs;

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let MapCallArgs {
            function: mapper,
            iterables,
            strict,
        } = args.bind_for(vm, "map")?;
        let iterators = iterables.into_vec();
        if iterators.is_empty() {
            return Err(vm.new_type_error("map() must have at least two arguments."));
        }
        let payload = Self {
            mapper,
            iterators,
            strict: Radium::new(strict),
        };
        payload.into_ref_with_type(vm, cls).map(Into::into)
    }

    fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        let PyMapNewArgs {
            function,
            iterable,
            iterables,
            strict,
        } = args;
        let _ = (function, iterable, iterables, strict);
        Err(vm.new_type_error("use slot_new"))
    }
}

#[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
impl PyMap {
    #[pymethod]
    fn __reduce__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
        let cls = zelf.class().to_owned();
        let mut vec = vec![zelf.mapper.clone()];
        vec.extend(zelf.iterators.iter().map(|o| o.clone().into()));
        let tuple_args = vm.ctx.new_tuple(vec);
        if zelf.strict.load(atomic::Ordering::Acquire) {
            vm.new_tuple((cls, tuple_args, true))
        } else {
            vm.new_tuple((cls, tuple_args))
        }
    }

    #[pymethod]
    fn __setstate__(zelf: PyRef<Self>, object: PyObjectRef, vm: &VirtualMachine) {
        if let Ok(obj) = ArgIntoBool::try_from_object(vm, object) {
            zelf.strict.store(obj.into(), atomic::Ordering::Release);
        }
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
