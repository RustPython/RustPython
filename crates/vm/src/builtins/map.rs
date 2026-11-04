use super::{
    PyType, PyTypeRef,
    zip::{get_iterators, next_item, release_iterables, strict_value},
};
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::PyTupleRef,
    class::PyClassImpl,
    function::{FuncArgs, OptionalArg, PosArgs},
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
    iterable: PyObjectRef,
    #[pyarg(flatten)]
    iterables: PosArgs<PyObjectRef, crate::function::NameIterables>,
    #[pyarg(named, default, py_default = "False")]
    strict: OptionalArg<PyObjectRef>,
}

#[derive(FromArgs)]
struct MapCallArgs {
    #[pyarg(positional, optional)]
    function: OptionalArg<PyObjectRef>,
    #[pyarg(flatten)]
    iterables: PosArgs<PyObjectRef, crate::function::NameIterables>,
    #[pyarg(named, default, py_default = "False")]
    strict: OptionalArg<PyObjectRef>,
}

impl Constructor for PyMap {
    type Args = PyMapNewArgs;

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let MapCallArgs {
            function,
            iterables,
            strict,
        } = args.bind_for(vm, "map")?;
        let result = (|| {
            let strict = strict_value(&strict, vm)?;
            let mapper = function
                .as_ref()
                .into_option()
                .filter(|_| !iterables.as_ref().is_empty())
                .ok_or_else(|| vm.new_type_error("map() must have at least two arguments."))?;
            let payload = Self::from_iterables(mapper, iterables.iter(), strict, vm)?;
            payload.into_ref_with_type(vm, cls).map(Into::into)
        })();
        release_iterables(strict, iterables);
        drop(function);
        result
    }

    fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        let Self::Args {
            function,
            iterable,
            iterables,
            strict,
        } = args;
        let result = (|| {
            let strict = strict_value(&strict, vm)?;
            Self::from_iterables(
                &function,
                core::iter::once(&iterable).chain(iterables.iter()),
                strict,
                vm,
            )
        })();
        release_iterables(strict, iterables);
        drop(iterable);
        drop(function);
        result
    }
}

impl PyMap {
    fn from_iterables<'a>(
        mapper: &PyObjectRef,
        iterables: impl IntoIterator<Item = &'a PyObjectRef>,
        strict: bool,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let iterators = get_iterators(iterables, vm)?;
        Ok(Self {
            mapper: mapper.clone(),
            iterators,
            strict: Radium::new(strict),
        })
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
    fn __setstate__(zelf: PyRef<Self>, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let strict = object.try_to_bool(vm)?;
        zelf.strict.store(strict, atomic::Ordering::Release);
        Ok(())
    }
}

impl SelfIter for PyMap {}

impl IterNext for PyMap {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let mut next_objs = Vec::with_capacity(zelf.iterators.len());
        for idx in 0..zelf.iterators.len() {
            let item =
                crate::raise_if_stop!(next_item(&zelf.iterators, idx, &zelf.strict, "map", vm)?);
            next_objs.push(item);
        }

        // the mapper itself can raise StopIteration which does stop the map iteration
        PyIterReturn::from_pyresult(zelf.mapper.call(next_objs, vm), vm)
    }
}

pub(crate) fn init(context: &'static Context) {
    PyMap::extend_class(context, context.types.map_type);
}
