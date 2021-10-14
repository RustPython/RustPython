//! Object Protocol
//! https://docs.python.org/3/c-api/object.html

use crate::{
    builtins::{pystr::IntoPyStrRef, PyBytes, PyInt, PyStrRef, PyTupleRef},
    bytesinner::ByteInnerNewOptions,
    common::{hash::PyHash, str::to_ascii},
    function::{IntoPyObject, OptionalArg},
    protocol::PyIter,
    pyref_type_error,
    types::{Constructor, PyComparisonOp},
    utils::Either,
    IdProtocol, PyArithmeticValue, PyObjectRef, PyResult, TryFromObject, TypeProtocol,
    VirtualMachine,
};

// RustPython doesn't need these items
// PyObject *Py_NotImplemented
// Py_RETURN_NOTIMPLEMENTED

impl PyObjectRef {
    // int PyObject_Print(PyObject *o, FILE *fp, int flags)

    pub fn has_attr(self, attr_name: impl IntoPyStrRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.get_attr(attr_name, vm).map(|o| vm.is_none(&o))
    }

    // get_attribute should be used for full attribute access (usually from user code).
    #[cfg_attr(feature = "flame-it", flame("PyObjectRef"))]
    pub fn get_attr(self, attr_name: impl IntoPyStrRef, vm: &VirtualMachine) -> PyResult {
        let attr_name = attr_name.into_pystr_ref(vm);
        vm_trace!("object.__getattribute__: {:?} {:?}", obj, attr_name);
        let getattro = self
            .class()
            .mro_find_map(|cls| cls.slots.getattro.load())
            .unwrap();
        getattro(self.clone(), attr_name.clone(), vm).map_err(|exc| {
            vm.set_attribute_error_context(&exc, self, attr_name);
            exc
        })
    }

    pub fn call_set_attr(
        &self,
        vm: &VirtualMachine,
        attr_name: PyStrRef,
        attr_value: Option<PyObjectRef>,
    ) -> PyResult<()> {
        let setattro = {
            let cls = self.class();
            cls.mro_find_map(|cls| cls.slots.setattro.load())
                .ok_or_else(|| {
                    let assign = attr_value.is_some();
                    let has_getattr = cls.mro_find_map(|cls| cls.slots.getattro.load()).is_some();
                    vm.new_type_error(format!(
                        "'{}' object has {} attributes ({} {})",
                        cls.name(),
                        if has_getattr { "only read-only" } else { "no" },
                        if assign { "assign to" } else { "del" },
                        attr_name
                    ))
                })?
        };
        setattro(self, attr_name, attr_value, vm)
    }

    // PyObject *PyObject_GenericGetAttr(PyObject *o, PyObject *name)

    pub fn set_attr(
        &self,
        attr_name: impl IntoPyStrRef,
        attr_value: impl Into<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let attr_name = attr_name.into_pystr_ref(vm);
        self.call_set_attr(vm, attr_name, Some(attr_value.into()))
    }

    // int PyObject_GenericSetAttr(PyObject *o, PyObject *name, PyObject *value)

    pub fn del_attr(&self, attr_name: impl IntoPyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        let attr_name = attr_name.into_pystr_ref(vm);
        self.call_set_attr(vm, attr_name, None)
    }

    // Perform a comparison, raising TypeError when the requested comparison
    // operator is not supported.
    // see: CPython PyObject_RichCompare
    fn _cmp(
        &self,
        other: &Self,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<Either<PyObjectRef, bool>> {
        let swapped = op.swapped();
        let call_cmp = |obj: &PyObjectRef, other, op| {
            let cmp = obj
                .class()
                .mro_find_map(|cls| cls.slots.richcompare.load())
                .unwrap();
            Ok(match cmp(obj, other, op, vm)? {
                Either::A(obj) => PyArithmeticValue::from_object(vm, obj).map(Either::A),
                Either::B(arithmetic) => arithmetic.map(Either::B),
            })
        };

        let mut checked_reverse_op = false;
        let is_strict_subclass = {
            let self_class = self.class();
            let other_class = other.class();
            !self_class.is(&other_class) && other_class.issubclass(&self_class)
        };
        if is_strict_subclass {
            let res = vm.with_recursion("in comparison", || call_cmp(other, self, swapped))?;
            checked_reverse_op = true;
            if let PyArithmeticValue::Implemented(x) = res {
                return Ok(x);
            }
        }
        if let PyArithmeticValue::Implemented(x) =
            vm.with_recursion("in comparison", || call_cmp(self, other, op))?
        {
            return Ok(x);
        }
        if !checked_reverse_op {
            let res = vm.with_recursion("in comparison", || call_cmp(other, self, swapped))?;
            if let PyArithmeticValue::Implemented(x) = res {
                return Ok(x);
            }
        }
        match op {
            PyComparisonOp::Eq => Ok(Either::B(self.is(&other))),
            PyComparisonOp::Ne => Ok(Either::B(!self.is(&other))),
            _ => Err(vm.new_unsupported_binop_error(self, other, op.operator_token())),
        }
    }

    // PyObject *PyObject_GenericGetDict(PyObject *o, void *context)
    // int PyObject_GenericSetDict(PyObject *o, PyObject *value, void *context)

    pub fn rich_compare(self, other: Self, opid: PyComparisonOp, vm: &VirtualMachine) -> PyResult {
        self._cmp(&other, opid, vm).map(|res| res.into_pyobject(vm))
    }

    pub fn rich_compare_bool(
        &self,
        other: &Self,
        opid: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        match self._cmp(other, opid, vm)? {
            Either::A(obj) => obj.try_to_bool(vm),
            Either::B(other) => Ok(other),
        }
    }

    pub fn repr(&self, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        vm.to_repr(self)
    }

    pub fn ascii(&self, vm: &VirtualMachine) -> PyResult<ascii::AsciiString> {
        let repr = vm.to_repr(self)?;
        let ascii = to_ascii(repr.as_str());
        Ok(ascii)
    }

    // Container of the virtual machine state:
    pub fn str(&self, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        if self.class().is(&vm.ctx.types.str_type) {
            Ok(self.clone().downcast().unwrap())
        } else {
            let s = vm.call_special_method(self.clone(), "__str__", ())?;
            s.try_into_value(vm)
        }
    }

    pub fn bytes(self, vm: &VirtualMachine) -> PyResult {
        let bytes_type = &vm.ctx.types.bytes_type;
        match self.downcast_exact::<PyInt>(vm) {
            Ok(int) => Err(pyref_type_error(vm, bytes_type, int.as_object())),
            Err(obj) => PyBytes::py_new(
                bytes_type.clone(),
                ByteInnerNewOptions {
                    source: OptionalArg::Present(obj),
                    encoding: OptionalArg::Missing,
                    errors: OptionalArg::Missing,
                },
                vm,
            ),
        }
    }

    pub fn is_subclass(&self, cls: &PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        vm.issubclass(self, cls)
    }

    /// Determines if `self` is an instance of `cls`, either directly, indirectly or virtually via
    /// the __instancecheck__ magic method.
    pub fn is_instance(&self, cls: &PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        // cpython first does an exact check on the type, although documentation doesn't state that
        // https://github.com/python/cpython/blob/a24107b04c1277e3c1105f98aff5bfa3a98b33a0/Objects/abstract.c#L2408
        if self.class().is(cls) {
            return Ok(true);
        }

        if cls.class().is(&vm.ctx.types.type_type) {
            return vm.abstract_isinstance(self, cls);
        }

        if let Ok(tuple) = PyTupleRef::try_from_object(vm, cls.clone()) {
            for typ in tuple.as_slice().iter() {
                if vm.with_recursion("in __instancecheck__", || self.is_instance(typ, vm))? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }

        if let Ok(meth) = vm.get_special_method(cls.clone(), "__instancecheck__")? {
            let ret =
                vm.with_recursion("in __instancecheck__", || meth.invoke((self.clone(),), vm))?;
            return ret.try_to_bool(vm);
        }

        vm.abstract_isinstance(self, cls)
    }

    pub fn hash(&self, vm: &VirtualMachine) -> PyResult<PyHash> {
        let hash = self
            .class()
            .mro_find_map(|cls| cls.slots.hash.load())
            .unwrap(); // hash always exist
        self.with_ptr(|zelf| hash(zelf, vm))
    }

    // const hash_not_implemented: fn(&PyObjectRef, &VirtualMachine) ->PyResult<PyHash> = crate::types::Unhashable::slot_hash;

    pub fn is_true(self, vm: &VirtualMachine) -> PyResult<bool> {
        self.try_to_bool(vm)
    }

    pub fn not(self, vm: &VirtualMachine) -> PyResult<bool> {
        self.is_true(vm).map(|x| !x)
    }

    // type protocol
    // PyObject *PyObject_Type(PyObject *o)

    // int PyObject_TypeCheck(PyObject *o, PyTypeObject *type)

    pub fn length(&self, vm: &VirtualMachine) -> PyResult<usize> {
        vm.obj_len_opt(self).unwrap_or_else(|| {
            Err(vm.new_type_error(format!(
                "object of type '{}' has no len()",
                self.class().name()
            )))
        })
    }

    pub fn length_hint(
        self,
        defaultvalue: Option<usize>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<usize>> {
        Ok(vm.length_hint(self)?.or(defaultvalue))
    }

    // item protocol
    // PyObject *PyObject_GetItem(PyObject *o, PyObject *key)
    // int PyObject_SetItem(PyObject *o, PyObject *key, PyObject *v)
    // int PyObject_DelItem(PyObject *o, PyObject *key)

    // PyObject *PyObject_Dir(PyObject *o)

    /// Takes an object and returns an iterator for it.
    /// This is typically a new iterator but if the argument is an iterator, this
    /// returns itself.
    pub fn get_iter(self, vm: &VirtualMachine) -> PyResult<PyIter> {
        // PyObject_GetIter
        PyIter::try_from_object(vm, self)
    }

    // PyObject *PyObject_GetAIter(PyObject *o)
}
