//! The `FrameLocalsProxy` type returned by `frame.f_locals` for optimized
//! (function) frames. Implements PEP 667 write-through semantics on top of the
//! frame's fast-local slots and an extra-locals side dict.

use super::{PyDict, PyDictRef, PyType};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    atomic_func,
    class::PyClassImpl,
    frame::FrameRef,
    function::{FuncArgs, OptionalArg, PyArithmeticValue, PyComparisonValue},
    object::{Traverse, TraverseFn},
    protocol::{PyMappingMethods, PyNumberMethods, PySequenceMethods},
    recursion::ReprGuard,
    types::{
        AsMapping, AsNumber, AsSequence, Comparable, Constructor, Iterable, PyComparisonOp,
        Representable,
    },
};
use rustpython_common::lock::LazyLock;
use rustpython_common::wtf8::Wtf8Buf;

#[pyclass(module = false, name = "FrameLocalsProxy", traverse = "manual")]
#[derive(Debug)]
pub struct FrameLocalsProxy {
    frame: FrameRef,
}

unsafe impl Traverse for FrameLocalsProxy {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.frame.traverse(tracer_fn);
    }
}

impl PyPayload for FrameLocalsProxy {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.frame_locals_proxy_type
    }
}

impl FrameLocalsProxy {
    pub(crate) fn new(frame: FrameRef) -> Self {
        Self { frame }
    }

    fn snapshot(&self, vm: &VirtualMachine) -> PyResult<PyDictRef> {
        self.frame.framelocalsproxy_snapshot(vm)
    }

    fn keys_vec(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        Ok(self.snapshot(vm)?.into_iter().map(|(k, _)| k).collect())
    }
}

impl Constructor for FrameLocalsProxy {
    type Args = FuncArgs;

    fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        if !args.kwargs.is_empty() {
            return Err(vm.new_type_error("FrameLocalsProxy() takes no keyword arguments"));
        }
        let mut args = args.args;
        if args.len() != 1 {
            return Err(vm.new_type_error(format!(
                "FrameLocalsProxy expected 1 argument, got {}",
                args.len()
            )));
        }
        let frame: FrameRef = args
            .pop()
            .unwrap()
            .downcast()
            .map_err(|_| vm.new_type_error("FrameLocalsProxy expected a frame"))?;
        Ok(Self::new(frame))
    }
}

#[pyclass(with(
    Constructor,
    AsMapping,
    AsSequence,
    AsNumber,
    Iterable,
    Comparable,
    Representable
))]
impl FrameLocalsProxy {
    fn __getitem__(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.frame.framelocalsproxy_getitem(key, vm)
    }

    fn __setitem__(&self, key: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.frame.framelocalsproxy_setitem(key, value, vm)
    }

    fn __delitem__(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.frame.framelocalsproxy_delitem(key, vm)
    }

    fn __contains__(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.frame.framelocalsproxy_contains(key, vm)
    }

    fn __len__(&self, vm: &VirtualMachine) -> PyResult<usize> {
        Ok(self.snapshot(vm)?.__len__())
    }

    #[pymethod]
    fn keys(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(vm.ctx.new_list(self.keys_vec(vm)?).into())
    }

    #[pymethod]
    fn values(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let values = self.snapshot(vm)?.into_iter().map(|(_, v)| v).collect();
        Ok(vm.ctx.new_list(values).into())
    }

    #[pymethod]
    fn items(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let items = self
            .snapshot(vm)?
            .into_iter()
            .map(|(k, v)| vm.ctx.new_tuple(vec![k, v]).into())
            .collect();
        Ok(vm.ctx.new_list(items).into())
    }

    #[pymethod]
    fn get(&self, key: PyObjectRef, default: OptionalArg, vm: &VirtualMachine) -> PyResult {
        match self.frame.framelocalsproxy_getitem(key, vm) {
            Ok(value) => Ok(value),
            Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => {
                Ok(default.unwrap_or_none(vm))
            }
            Err(e) => Err(e),
        }
    }

    #[pymethod]
    fn pop(&self, key: PyObjectRef, default: OptionalArg, vm: &VirtualMachine) -> PyResult {
        self.frame
            .framelocalsproxy_pop(key, default.into_option(), vm)
    }

    #[pymethod]
    fn setdefault(
        &self,
        key: PyObjectRef,
        default: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult {
        self.frame
            .framelocalsproxy_setdefault(key, default.unwrap_or_none(vm), vm)
    }

    #[pymethod]
    fn copy(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(self.snapshot(vm)?.into())
    }

    #[pymethod]
    fn update(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        self.update_from(&other, vm)
    }

    fn update_from(&self, other: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        let items: Vec<(PyObjectRef, PyObjectRef)> =
            if let Some(dict) = other.downcast_ref::<PyDict>() {
                dict.into_iter().collect()
            } else if let Some(proxy) = other.downcast_ref::<Self>() {
                proxy.snapshot(vm)?.into_iter().collect()
            } else {
                return Err(vm.new_type_error(
                    "update() argument must be dict or another FrameLocalsProxy",
                ));
            };
        for (key, value) in items {
            self.frame.framelocalsproxy_setitem(key, value, vm)?;
        }
        Ok(())
    }

    #[pymethod]
    fn __reversed__(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let mut keys = self.keys_vec(vm)?;
        keys.reverse();
        Ok(vm.ctx.new_list(keys).into())
    }

    fn __ior__(zelf: PyRef<Self>, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        zelf.update_from(&other, vm)?;
        Ok(zelf.into())
    }

    fn __or__(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let base = self.snapshot(vm)?;
        vm._or(base.as_object(), &other)
    }

    fn __ror__(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let base = self.snapshot(vm)?;
        vm._or(&other, base.as_object())
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("cannot pickle 'FrameLocalsProxy' object"))
    }

    #[pymethod]
    fn __reduce_ex__(&self, _protocol: OptionalArg, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("cannot pickle 'FrameLocalsProxy' object"))
    }
}

impl AsMapping for FrameLocalsProxy {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: LazyLock<PyMappingMethods> = LazyLock::new(|| PyMappingMethods {
            length: atomic_func!(|mapping, vm| FrameLocalsProxy::mapping_downcast(mapping).__len__(vm)),
            subscript: atomic_func!(|mapping, needle, vm| {
                FrameLocalsProxy::mapping_downcast(mapping).__getitem__(needle.to_owned(), vm)
            }),
            ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                let zelf = FrameLocalsProxy::mapping_downcast(mapping);
                match value {
                    Some(value) => zelf.__setitem__(needle.to_owned(), value, vm),
                    None => zelf.__delitem__(needle.to_owned(), vm),
                }
            }),
        });
        &AS_MAPPING
    }
}

impl AsSequence for FrameLocalsProxy {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
            contains: atomic_func!(|seq, target, vm| {
                FrameLocalsProxy::sequence_downcast(seq).__contains__(target.to_owned(), vm)
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl AsNumber for FrameLocalsProxy {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            or: Some(|a, b, vm| {
                if let Some(proxy) = a.downcast_ref::<FrameLocalsProxy>() {
                    proxy.__or__(b.to_owned(), vm)
                } else if let Some(proxy) = b.downcast_ref::<FrameLocalsProxy>() {
                    proxy.__ror__(a.to_owned(), vm)
                } else {
                    Ok(vm.ctx.not_implemented())
                }
            }),
            inplace_or: Some(|a, b, vm| {
                let proxy = a
                    .to_owned()
                    .downcast::<FrameLocalsProxy>()
                    .map_err(|_| vm.new_type_error("expected FrameLocalsProxy"))?;
                FrameLocalsProxy::__ior__(proxy, b.to_owned(), vm)
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl Iterable for FrameLocalsProxy {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let keys = vm.ctx.new_list(zelf.keys_vec(vm)?);
        keys.as_object().to_owned().get_iter(vm).map(Into::into)
    }
}

impl Comparable for FrameLocalsProxy {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| {
            let self_dict: PyObjectRef = zelf.snapshot(vm)?.into();
            let other_obj = match other.downcast_ref::<Self>() {
                Some(proxy) => proxy.snapshot(vm)?.into(),
                None => other.to_owned(),
            };
            let res = self_dict.rich_compare(other_obj, PyComparisonOp::Eq, vm)?;
            PyArithmeticValue::from_object(vm, res)
                .map(|o| o.try_to_bool(vm))
                .transpose()
        })
    }
}

impl Representable for FrameLocalsProxy {
    fn repr_wtf8(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
        if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            let dict = zelf.snapshot(vm)?;
            Ok(dict.as_object().repr(vm)?.as_wtf8().to_owned())
        } else {
            Ok(Wtf8Buf::from("{...}"))
        }
    }
}

pub(crate) fn init(context: &'static Context) {
    FrameLocalsProxy::extend_class(context, context.types.frame_locals_proxy_type);
}
