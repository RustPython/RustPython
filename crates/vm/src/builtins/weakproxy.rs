use super::{PyStr, PyStrRef, PyType, PyTypeRef, PyWeak};
use crate::common::lock::LazyLock;
use crate::{
    Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine, atomic_func,
    class::PyClassImpl,
    common::hash::PyHash,
    function::{OptionalArg, PyArithmeticValue, PyComparisonValue, PySetterValue},
    protocol::{PyIter, PyIterReturn, PyMappingMethods, PyNumberMethods, PySequenceMethods},
    stdlib::builtins::reversed,
    types::{
        AsMapping, AsNumber, AsSequence, Comparable, Constructor, GetAttr, Hashable, IterNext,
        Iterable, PyComparisonOp, Representable, SetAttr,
    },
};

#[pyclass(module = false, name = "weakproxy", unhashable = true, traverse)]
#[derive(Debug)]
pub struct PyWeakProxy {
    weak: PyRef<PyWeak>,
}

impl PyPayload for PyWeakProxy {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.weakproxy_type
    }
}

#[derive(FromArgs)]
pub struct WeakProxyNewArgs {
    #[pyarg(positional)]
    referent: PyObjectRef,
    #[pyarg(positional, optional)]
    callback: OptionalArg<PyObjectRef>,
}

impl Constructor for PyWeakProxy {
    type Args = WeakProxyNewArgs;

    fn py_new(
        _cls: &Py<PyType>,
        Self::Args { referent, callback }: Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        // using an internal subclass as the class prevents us from getting the generic weakref,
        // which would mess up the weakref count
        let weak_cls = WEAK_SUBCLASS.get_or_init(|| {
            vm.ctx.new_class(
                None,
                "__weakproxy",
                vm.ctx.types.weakref_type.to_owned(),
                super::PyWeak::make_slots(),
            )
        });
        // TODO: PyWeakProxy should use the same payload as PyWeak
        Ok(Self {
            weak: referent.downgrade_with_typ(callback.into_option(), weak_cls.clone(), vm)?,
        })
    }
}

crate::common::static_cell! {
    static WEAK_SUBCLASS: PyTypeRef;
}

#[pyclass(with(
    GetAttr,
    SetAttr,
    Constructor,
    Comparable,
    AsNumber,
    AsSequence,
    AsMapping,
    Representable,
    IterNext
))]
impl PyWeakProxy {
    fn try_upgrade(&self, vm: &VirtualMachine) -> PyResult {
        self.weak.upgrade().ok_or_else(|| new_reference_error(vm))
    }

    #[pymethod]
    fn __str__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        zelf.try_upgrade(vm)?.str(vm)
    }

    fn len(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.try_upgrade(vm)?.length(vm)
    }

    #[pymethod]
    fn __bytes__(&self, vm: &VirtualMachine) -> PyResult {
        self.try_upgrade(vm)?.bytes(vm)
    }

    #[pymethod]
    fn __reversed__(&self, vm: &VirtualMachine) -> PyResult {
        let obj = self.try_upgrade(vm)?;
        reversed(obj, vm)
    }
    fn __contains__(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.try_upgrade(vm)?
            .sequence_unchecked()
            .contains(&needle, vm)
    }

    fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let obj = self.try_upgrade(vm)?;
        obj.get_item(&*needle, vm)
    }

    fn setitem(
        &self,
        needle: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let obj = self.try_upgrade(vm)?;
        obj.set_item(&*needle, value, vm)
    }

    fn delitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let obj = self.try_upgrade(vm)?;
        obj.del_item(&*needle, vm)
    }
}

impl Iterable for PyWeakProxy {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let obj = zelf.try_upgrade(vm)?;
        Ok(obj.get_iter(vm)?.into())
    }
}

impl IterNext for PyWeakProxy {
    fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
        let obj = zelf.try_upgrade(vm)?;
        if obj.class().slots.iternext.load().is_none() {
            return Err(vm.new_type_error("Weakref proxy referenced a non-iterator".to_owned()));
        }
        PyIter::new(obj).next(vm)
    }
}

fn new_reference_error(vm: &VirtualMachine) -> PyRef<super::PyBaseException> {
    vm.new_exception_msg(
        vm.ctx.exceptions.reference_error.to_owned(),
        "weakly-referenced object no longer exists".into(),
    )
}

impl GetAttr for PyWeakProxy {
    // TODO: callbacks
    fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        let obj = zelf.try_upgrade(vm)?;
        obj.get_attr(name, vm)
    }
}

impl SetAttr for PyWeakProxy {
    fn setattro(
        zelf: &Py<Self>,
        attr_name: &Py<PyStr>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let obj = zelf.try_upgrade(vm)?;
        obj.call_set_attr(vm, attr_name, value)
    }
}

fn proxy_upgrade(obj: &PyObject, vm: &VirtualMachine) -> PyResult {
    obj.downcast_ref::<PyWeakProxy>()
        .expect("proxy_upgrade called on non-PyWeakProxy object")
        .try_upgrade(vm)
}

fn proxy_upgrade_opt(obj: &PyObject, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
    match obj.downcast_ref::<PyWeakProxy>() {
        Some(proxy) => Ok(Some(proxy.try_upgrade(vm)?)),
        None => Ok(None),
    }
}

fn proxy_unary_op(
    obj: &PyObject,
    vm: &VirtualMachine,
    op: fn(&VirtualMachine, &PyObject) -> PyResult,
) -> PyResult {
    let upgraded = proxy_upgrade(obj, vm)?;
    op(vm, &upgraded)
}

macro_rules! proxy_unary_slot {
    ($vm_method:ident) => {
        Some(|number, vm| proxy_unary_op(number.obj, vm, |vm, obj| vm.$vm_method(obj)))
    };
}

fn proxy_binary_op(
    a: &PyObject,
    b: &PyObject,
    vm: &VirtualMachine,
    op: fn(&VirtualMachine, &PyObject, &PyObject) -> PyResult,
) -> PyResult {
    let a_up = proxy_upgrade_opt(a, vm)?;
    let b_up = proxy_upgrade_opt(b, vm)?;
    let a_ref = a_up.as_deref().unwrap_or(a);
    let b_ref = b_up.as_deref().unwrap_or(b);
    op(vm, a_ref, b_ref)
}

macro_rules! proxy_binary_slot {
    ($vm_method:ident) => {
        Some(|a, b, vm| proxy_binary_op(a, b, vm, |vm, a, b| vm.$vm_method(a, b)))
    };
}

fn proxy_ternary_op(
    a: &PyObject,
    b: &PyObject,
    c: &PyObject,
    vm: &VirtualMachine,
    op: fn(&VirtualMachine, &PyObject, &PyObject, &PyObject) -> PyResult,
) -> PyResult {
    let a_up = proxy_upgrade_opt(a, vm)?;
    let b_up = proxy_upgrade_opt(b, vm)?;
    let c_up = proxy_upgrade_opt(c, vm)?;
    let a_ref = a_up.as_deref().unwrap_or(a);
    let b_ref = b_up.as_deref().unwrap_or(b);
    let c_ref = c_up.as_deref().unwrap_or(c);
    op(vm, a_ref, b_ref, c_ref)
}

macro_rules! proxy_ternary_slot {
    ($vm_method:ident) => {
        Some(|a, b, c, vm| proxy_ternary_op(a, b, c, vm, |vm, a, b, c| vm.$vm_method(a, b, c)))
    };
}

impl AsNumber for PyWeakProxy {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: LazyLock<PyNumberMethods> = LazyLock::new(|| PyNumberMethods {
            boolean: Some(|number, vm| {
                let obj = proxy_upgrade(number.obj, vm)?;
                obj.is_true(vm)
            }),
            int: Some(|number, vm| {
                let obj = proxy_upgrade(number.obj, vm)?;
                obj.try_int(vm).map(Into::into)
            }),
            float: Some(|number, vm| {
                let obj = proxy_upgrade(number.obj, vm)?;
                obj.try_float(vm).map(Into::into)
            }),
            index: Some(|number, vm| {
                let obj = proxy_upgrade(number.obj, vm)?;
                obj.try_index(vm).map(Into::into)
            }),
            negative: proxy_unary_slot!(_neg),
            positive: proxy_unary_slot!(_pos),
            absolute: proxy_unary_slot!(_abs),
            invert: proxy_unary_slot!(_invert),
            add: proxy_binary_slot!(_add),
            subtract: proxy_binary_slot!(_sub),
            multiply: proxy_binary_slot!(_mul),
            remainder: proxy_binary_slot!(_mod),
            divmod: proxy_binary_slot!(_divmod),
            lshift: proxy_binary_slot!(_lshift),
            rshift: proxy_binary_slot!(_rshift),
            and: proxy_binary_slot!(_and),
            xor: proxy_binary_slot!(_xor),
            or: proxy_binary_slot!(_or),
            floor_divide: proxy_binary_slot!(_floordiv),
            true_divide: proxy_binary_slot!(_truediv),
            matrix_multiply: proxy_binary_slot!(_matmul),
            inplace_add: proxy_binary_slot!(_iadd),
            inplace_subtract: proxy_binary_slot!(_isub),
            inplace_multiply: proxy_binary_slot!(_imul),
            inplace_remainder: proxy_binary_slot!(_imod),
            inplace_lshift: proxy_binary_slot!(_ilshift),
            inplace_rshift: proxy_binary_slot!(_irshift),
            inplace_and: proxy_binary_slot!(_iand),
            inplace_xor: proxy_binary_slot!(_ixor),
            inplace_or: proxy_binary_slot!(_ior),
            inplace_floor_divide: proxy_binary_slot!(_ifloordiv),
            inplace_true_divide: proxy_binary_slot!(_itruediv),
            inplace_matrix_multiply: proxy_binary_slot!(_imatmul),
            power: proxy_ternary_slot!(_pow),
            inplace_power: proxy_ternary_slot!(_ipow),
        });
        &AS_NUMBER
    }
}

impl Comparable for PyWeakProxy {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        let obj = zelf.try_upgrade(vm)?;
        // CPython parity (Objects/weakref.c::proxy_richcompare): delegate to
        // PyObject_RichCompare on the referent, not the bool variant.
        let res = obj.rich_compare(other.to_owned(), op, vm)?;
        PyArithmeticValue::from_object(vm, res)
            .map(|o| o.try_to_bool(vm))
            .transpose()
    }
}

impl AsSequence for PyWeakProxy {
    fn as_sequence() -> &'static PySequenceMethods {
        static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
            length: atomic_func!(|seq, vm| PyWeakProxy::sequence_downcast(seq).len(vm)),
            contains: atomic_func!(|seq, needle, vm| {
                PyWeakProxy::sequence_downcast(seq).__contains__(needle.to_owned(), vm)
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

impl AsMapping for PyWeakProxy {
    fn as_mapping() -> &'static PyMappingMethods {
        static AS_MAPPING: PyMappingMethods = PyMappingMethods {
            length: atomic_func!(|mapping, vm| PyWeakProxy::mapping_downcast(mapping).len(vm)),
            subscript: atomic_func!(|mapping, needle, vm| {
                PyWeakProxy::mapping_downcast(mapping).getitem(needle.to_owned(), vm)
            }),
            ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                let zelf = PyWeakProxy::mapping_downcast(mapping);
                if let Some(value) = value {
                    zelf.setitem(needle.to_owned(), value, vm)
                } else {
                    zelf.delitem(needle.to_owned(), vm)
                }
            }),
        };
        &AS_MAPPING
    }
}

impl Representable for PyWeakProxy {
    #[inline]
    fn repr(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        zelf.try_upgrade(vm)?.repr(vm)
    }

    #[cold]
    fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        unreachable!("use repr instead")
    }
}

pub(crate) fn init(context: &'static Context) {
    PyWeakProxy::extend_class(context, context.types.weakproxy_type);
}

impl Hashable for PyWeakProxy {
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        zelf.try_upgrade(vm)?.hash(vm)
    }
}
