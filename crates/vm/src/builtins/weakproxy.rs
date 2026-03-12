use super::{PyStr, PyStrRef, PyType, PyTypeRef, PyWeak};
use crate::common::lock::LazyLock;
use crate::{
    Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine, atomic_func,
    class::PyClassImpl,
    common::hash::PyHash,
    function::{OptionalArg, PyComparisonValue, PySetterValue},
    protocol::{PyIter, PyIterReturn, PyMappingMethods, PyNumber, PyNumberMethods, PySequenceMethods},
    stdlib::builtins::reversed,
    types::{
        AsMapping, AsNumber, AsSequence, Comparable, Constructor, GetAttr, Hashable,
        IterNext, Iterable, PyComparisonOp, Representable, SetAttr,
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
            return Err(vm.new_type_error(
                "Weakref proxy referenced a non-iterator".to_owned(),
            ));
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
        .unwrap()
        .try_upgrade(vm)
}

fn proxy_upgrade_opt(obj: &PyObject, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
    if obj.downcast_ref::<PyWeakProxy>().is_some() {
        Ok(Some(proxy_upgrade(obj, vm)?))
    } else {
        Ok(None)
    }
}

macro_rules! proxy_unary_slot {
    ($slot:ident) => {
        Some(|number, vm| {
            let obj = proxy_upgrade(number.obj, vm)?;
            let f = obj.class().slots.as_number.$slot.load()
                .ok_or_else(|| vm.new_type_error(format!(
                    "bad operand type for unary op: '{}'",
                    obj.class().name()
                )))?;
            let number = PyNumber { obj: &obj };
            f(number, vm).map(|v| v.into())
        })
    };
}

macro_rules! proxy_binary_slot {
    ($slot:ident) => {
        Some(|a, b, vm| {
            let a_up = proxy_upgrade_opt(a, vm)?;
            let b_up = proxy_upgrade_opt(b, vm)?;
            let a_ref = a_up.as_deref().unwrap_or(a);
            let b_ref = b_up.as_deref().unwrap_or(b);
            if let Some(f) = a_ref.class().slots.as_number.$slot.load() {
                f(a_ref, b_ref, vm)
            } else {
                Ok(vm.ctx.not_implemented())
            }
        })
    };
    ($slot:ident, $right_slot:ident) => {
        Some(|a, b, vm| {
            let a_up = proxy_upgrade_opt(a, vm)?;
            let b_up = proxy_upgrade_opt(b, vm)?;
            let a_ref = a_up.as_deref().unwrap_or(a);
            let b_ref = b_up.as_deref().unwrap_or(b);
            if a_up.is_some() {
                // Proxy on the left: use forward slot
                if let Some(f) = a_ref.class().slots.as_number.$slot.load() {
                    f(a_ref, b_ref, vm)
                } else {
                    Ok(vm.ctx.not_implemented())
                }
            } else {
                // Proxy on the right: use right slot
                if let Some(f) = b_ref.class().slots.as_number.$right_slot.load() {
                    f(a_ref, b_ref, vm)
                } else {
                    Ok(vm.ctx.not_implemented())
                }
            }
        })
    };
}

macro_rules! proxy_ternary_slot {
    ($slot:ident) => {
        Some(|a, b, c, vm| {
            let a_up = proxy_upgrade_opt(a, vm)?;
            let b_up = proxy_upgrade_opt(b, vm)?;
            let c_up = proxy_upgrade_opt(c, vm)?;
            let a_ref = a_up.as_deref().unwrap_or(a);
            let b_ref = b_up.as_deref().unwrap_or(b);
            let c_ref = c_up.as_deref().unwrap_or(c);
            if let Some(f) = a_ref.class().slots.as_number.$slot.load() {
                f(a_ref, b_ref, c_ref, vm)
            } else {
                Ok(vm.ctx.not_implemented())
            }
        })
    };
    ($slot:ident, $right_slot:ident) => {
        Some(|a, b, c, vm| {
            let a_up = proxy_upgrade_opt(a, vm)?;
            let b_up = proxy_upgrade_opt(b, vm)?;
            let c_up = proxy_upgrade_opt(c, vm)?;
            let a_ref = a_up.as_deref().unwrap_or(a);
            let b_ref = b_up.as_deref().unwrap_or(b);
            let c_ref = c_up.as_deref().unwrap_or(c);
            if a_up.is_some() {
                if let Some(f) = a_ref.class().slots.as_number.$slot.load() {
                    f(a_ref, b_ref, c_ref, vm)
                } else {
                    Ok(vm.ctx.not_implemented())
                }
            } else {
                if let Some(f) = b_ref.class().slots.as_number.$right_slot.load() {
                    f(a_ref, b_ref, c_ref, vm)
                } else {
                    Ok(vm.ctx.not_implemented())
                }
            }
        })
    };
}

impl AsNumber for PyWeakProxy {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: LazyLock<PyNumberMethods> = LazyLock::new(|| PyNumberMethods {
            boolean: Some(|number, vm| {
                let zelf = number.obj.downcast_ref::<PyWeakProxy>().unwrap();
                zelf.try_upgrade(vm)?.is_true(vm)
            }),
            int: proxy_unary_slot!(int),
            float: proxy_unary_slot!(float),
            index: proxy_unary_slot!(index),
            negative: proxy_unary_slot!(negative),
            positive: proxy_unary_slot!(positive),
            absolute: proxy_unary_slot!(absolute),
            invert: proxy_unary_slot!(invert),
            add: proxy_binary_slot!(add, right_add),
            subtract: proxy_binary_slot!(subtract, right_subtract),
            multiply: proxy_binary_slot!(multiply, right_multiply),
            remainder: proxy_binary_slot!(remainder, right_remainder),
            divmod: proxy_binary_slot!(divmod, right_divmod),
            lshift: proxy_binary_slot!(lshift, right_lshift),
            rshift: proxy_binary_slot!(rshift, right_rshift),
            and: proxy_binary_slot!(and, right_and),
            xor: proxy_binary_slot!(xor, right_xor),
            or: proxy_binary_slot!(or, right_or),
            floor_divide: proxy_binary_slot!(floor_divide, right_floor_divide),
            true_divide: proxy_binary_slot!(true_divide, right_true_divide),
            matrix_multiply: proxy_binary_slot!(matrix_multiply, right_matrix_multiply),
            inplace_add: proxy_binary_slot!(inplace_add),
            inplace_subtract: proxy_binary_slot!(inplace_subtract),
            inplace_multiply: proxy_binary_slot!(inplace_multiply),
            inplace_remainder: proxy_binary_slot!(inplace_remainder),
            inplace_lshift: proxy_binary_slot!(inplace_lshift),
            inplace_rshift: proxy_binary_slot!(inplace_rshift),
            inplace_and: proxy_binary_slot!(inplace_and),
            inplace_xor: proxy_binary_slot!(inplace_xor),
            inplace_or: proxy_binary_slot!(inplace_or),
            inplace_floor_divide: proxy_binary_slot!(inplace_floor_divide),
            inplace_true_divide: proxy_binary_slot!(inplace_true_divide),
            inplace_matrix_multiply: proxy_binary_slot!(inplace_matrix_multiply),
            power: proxy_ternary_slot!(power, right_power),
            inplace_power: proxy_ternary_slot!(inplace_power),
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
        Ok(PyComparisonValue::Implemented(
            obj.rich_compare_bool(other, op, vm)?,
        ))
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

pub fn init(context: &'static Context) {
    PyWeakProxy::extend_class(context, context.types.weakproxy_type);
}

impl Hashable for PyWeakProxy {
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        zelf.try_upgrade(vm)?.hash(vm)
    }
}
