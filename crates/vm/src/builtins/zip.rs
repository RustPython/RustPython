use super::PyType;
use crate::{
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::PyTupleRef,
    class::PyClassImpl,
    function::{OptionalArg, PosArgs},
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

#[derive(FromArgs)]
pub struct PyZipNewArgs {
    #[pyarg(flatten)]
    iterables: PosArgs<PyObjectRef, crate::function::NameIterables>,
    #[pyarg(named, default, py_default = "False")]
    strict: OptionalArg<PyObjectRef>,
}

impl Constructor for PyZip {
    type Args = PyZipNewArgs;

    fn py_new(
        _cls: &Py<PyType>,
        Self::Args { iterables, strict }: Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let result = (|| {
            let strict = strict_value(&strict, vm)?;
            let iterators = get_iterators(iterables.iter(), vm)?;
            Ok(Self {
                iterators,
                strict: Radium::new(strict),
            })
        })();
        release_iterables(strict, iterables);
        result
    }
}

#[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
impl PyZip {
    #[pymethod]
    fn __reduce__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
        let cls = zelf.class().to_owned();
        let iterators = zelf
            .iterators
            .iter()
            .map(|obj| obj.clone().into())
            .collect::<Vec<_>>();
        let tuple_iter = vm.ctx.new_tuple(iterators);
        if zelf.strict.load(atomic::Ordering::Acquire) {
            vm.new_tuple((cls, tuple_iter, true))
        } else {
            vm.new_tuple((cls, tuple_iter))
        }
    }

    #[pymethod]
    fn __setstate__(zelf: PyRef<Self>, object: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let strict = object.try_to_bool(vm)?;
        zelf.strict.store(strict, atomic::Ordering::Release);
        Ok(())
    }
}

impl SelfIter for PyZip {}

impl IterNext for PyZip {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        if zelf.iterators.is_empty() {
            return Ok(PyIterReturn::StopIteration(None));
        }
        let mut next_objs = Vec::with_capacity(zelf.iterators.len());
        for idx in 0..zelf.iterators.len() {
            let item =
                crate::raise_if_stop!(next_item(&zelf.iterators, idx, &zelf.strict, "zip", vm)?);
            next_objs.push(item);
        }
        Ok(PyIterReturn::Return(vm.ctx.new_tuple(next_objs).into()))
    }
}

pub(super) fn strict_value(
    strict: &OptionalArg<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<bool> {
    strict
        .as_ref()
        .map(|value| value.try_to_bool(vm))
        .transpose()
        .map(|value| value.unwrap_or(false))
}

pub(super) fn get_iterators<'a>(
    iterables: impl IntoIterator<Item = &'a PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<Vec<PyIter>> {
    let iterables = iterables.into_iter();
    let mut iterators = Vec::with_capacity(iterables.size_hint().0);
    for obj in iterables {
        match obj.get_iter(vm) {
            Ok(iterator) => iterators.push(iterator),
            Err(error) => {
                while let Some(iterator) = iterators.pop() {
                    drop(iterator);
                }
                return Err(error);
            }
        }
    }
    Ok(iterators)
}

// Constructor callbacks borrow the complete argument list. Release it only
// afterwards, in call-stack order, including when conversion failed.
pub(super) fn release_iterables(
    strict: OptionalArg<PyObjectRef>,
    iterables: PosArgs<PyObjectRef, crate::function::NameIterables>,
) {
    drop(strict);
    iterables.into_iter().rev().for_each(drop);
}

// Read strict after the source callback, but before clearing a StopIteration
// exception: either operation can run user code that changes the flag.
pub(super) fn next_item(
    iterators: &[PyIter],
    index: usize,
    strict: &PyAtomic<bool>,
    name: &str,
    vm: &VirtualMachine,
) -> PyResult<PyIterReturn> {
    let result = iterators[index].next(vm);
    if matches!(&result, Ok(PyIterReturn::Return(_))) || !strict.load(atomic::Ordering::Acquire) {
        return result;
    }
    // Native iterators can surface StopIteration as an exception as well as
    // the dedicated return variant (for example, a filter item's __bool__).
    // Preserve the original exception in non-strict mode.
    match result.or_else(|error| PyIterReturn::from_pyresult(Err(error), vm))? {
        PyIterReturn::StopIteration(value) => {
            drop(value);
            strict_exhausted(iterators, index, name, vm)
        }
        item => Ok(item),
    }
}

// Both map and zip check the remaining sources only when the first one stops.
// Callers clear its StopIteration value before checking: strict exhaustion does
// not propagate source return values, unlike ordinary shortest-input iteration.
fn strict_exhausted(
    iterators: &[PyIter],
    stopped: usize,
    name: &str,
    vm: &VirtualMachine,
) -> PyResult<PyIterReturn> {
    let (index, direction) = if stopped > 0 {
        (stopped, "shorter")
    } else {
        let mut longer = None;
        for (index, iterator) in iterators.iter().enumerate().skip(1) {
            if let PyIterReturn::Return(item) = iterator
                .next(vm)
                .or_else(|error| PyIterReturn::from_pyresult(Err(error), vm))?
            {
                drop(item);
                longer = Some(index);
                break;
            }
        }
        let Some(index) = longer else {
            return Ok(PyIterReturn::StopIteration(None));
        };
        (index, "longer")
    };
    let plural = if index == 1 { " " } else { "s 1-" };
    Err(vm.new_value_error(format!(
        "{name}() argument {} is {direction} than argument{plural}{index}",
        index + 1,
    )))
}

pub(crate) fn init(ctx: &'static Context) {
    PyZip::extend_class(ctx, ctx.types.zip_type);
}
