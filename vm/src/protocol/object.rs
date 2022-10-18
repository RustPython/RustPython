//! Object Protocol
//! https://docs.python.org/3/c-api/object.html

use crate::{
    builtins::{
        pystr::IntoPyStrRef, PyBytes, PyDict, PyDictRef, PyGenericAlias, PyInt, PyStrRef,
        PyTupleRef, PyTypeRef,
    },
    bytesinner::ByteInnerNewOptions,
    common::{hash::PyHash, str::to_ascii},
    convert::{ToPyObject, ToPyResult},
    dictdatatype::DictKey,
    function::{Either, OptionalArg, PyArithmeticValue, PySetterValue},
    protocol::{PyIter, PyMapping, PySequence},
    types::{Constructor, PyComparisonOp},
    AsObject, PyObject, PyObjectRef, PyResult, TryFromObject, VirtualMachine,
};

// RustPython doesn't need these items
// PyObject *Py_NotImplemented
// Py_RETURN_NOTIMPLEMENTED

impl PyObjectRef {
    // int PyObject_Print(PyObject *o, FILE *fp, int flags)

    // PyObject *PyObject_GenericGetDict(PyObject *o, void *context)
    // int PyObject_GenericSetDict(PyObject *o, PyObject *value, void *context)

    #[inline(always)]
    pub fn rich_compare(self, other: Self, opid: PyComparisonOp, vm: &VirtualMachine) -> PyResult {
        self._cmp(&other, opid, vm).map(|res| res.to_pyobject(vm))
    }

    pub fn bytes(self, vm: &VirtualMachine) -> PyResult {
        let bytes_type = vm.ctx.types.bytes_type;
        match self.downcast_exact::<PyInt>(vm) {
            Ok(int) => Err(vm.new_downcast_type_error(bytes_type, &int)),
            Err(obj) => PyBytes::py_new(
                bytes_type.to_owned(),
                ByteInnerNewOptions {
                    source: OptionalArg::Present(obj),
                    encoding: OptionalArg::Missing,
                    errors: OptionalArg::Missing,
                },
                vm,
            ),
        }
    }

    // const hash_not_implemented: fn(&PyObject, &VirtualMachine) ->PyResult<PyHash> = crate::types::Unhashable::slot_hash;

    pub fn is_true(self, vm: &VirtualMachine) -> PyResult<bool> {
        self.try_to_bool(vm)
    }

    pub fn not(self, vm: &VirtualMachine) -> PyResult<bool> {
        self.is_true(vm).map(|x| !x)
    }

    pub fn length_hint(self, defaultvalue: usize, vm: &VirtualMachine) -> PyResult<usize> {
        Ok(vm.length_hint_opt(self)?.unwrap_or(defaultvalue))
    }

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

impl PyObject {
    pub fn has_attr(&self, attr_name: impl IntoPyStrRef, vm: &VirtualMachine) -> PyResult<bool> {
        self.get_attr(attr_name, vm).map(|o| vm.is_none(&o))
    }

    pub fn get_attr(&self, attr_name: impl IntoPyStrRef, vm: &VirtualMachine) -> PyResult {
        let attr_name = attr_name.into_pystr_ref(vm);
        self._get_attr(attr_name, vm)
    }

    // get_attribute should be used for full attribute access (usually from user code).
    #[cfg_attr(feature = "flame-it", flame("PyObjectRef"))]
    #[inline]
    fn _get_attr(&self, attr_name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        vm_trace!("object.__getattribute__: {:?} {:?}", self, attr_name);
        let getattro = self
            .class()
            .mro_find_map(|cls| cls.slots.getattro.load())
            .unwrap();
        getattro(self, attr_name.clone(), vm).map_err(|exc| {
            vm.set_attribute_error_context(&exc, self.to_owned(), attr_name);
            exc
        })
    }

    pub fn call_set_attr(
        &self,
        vm: &VirtualMachine,
        attr_name: PyStrRef,
        attr_value: PySetterValue,
    ) -> PyResult<()> {
        let setattro = {
            let cls = self.class();
            cls.mro_find_map(|cls| cls.slots.setattro.load())
                .ok_or_else(|| {
                    let has_getattr = cls.mro_find_map(|cls| cls.slots.getattro.load()).is_some();
                    vm.new_type_error(format!(
                        "'{}' object has {} attributes ({} {})",
                        cls.name(),
                        if has_getattr { "only read-only" } else { "no" },
                        if attr_value.is_assign() {
                            "assign to"
                        } else {
                            "del"
                        },
                        attr_name
                    ))
                })?
        };
        setattro(self, attr_name, attr_value, vm)
    }

    pub fn set_attr(
        &self,
        attr_name: impl IntoPyStrRef,
        attr_value: impl Into<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let attr_name = attr_name.into_pystr_ref(vm);
        self.call_set_attr(vm, attr_name, PySetterValue::Assign(attr_value.into()))
    }

    // int PyObject_GenericSetAttr(PyObject *o, PyObject *name, PyObject *value)
    #[cfg_attr(feature = "flame-it", flame)]
    pub fn generic_setattr(
        &self,
        attr_name: PyStrRef, // TODO: Py<PyStr>
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        vm_trace!("object.__setattr__({:?}, {}, {:?})", self, attr_name, value);
        if let Some(attr) = vm
            .ctx
            .interned_str(&*attr_name)
            .and_then(|attr_name| self.get_class_attr(attr_name))
        {
            let descr_set = attr.class().mro_find_map(|cls| cls.slots.descr_set.load());
            if let Some(descriptor) = descr_set {
                return descriptor(attr, self.to_owned(), value, vm);
            }
        }

        if let Some(dict) = self.dict() {
            if let PySetterValue::Assign(value) = value {
                dict.set_item(&*attr_name, value, vm)?;
            } else {
                dict.del_item(&*attr_name, vm).map_err(|e| {
                    if e.fast_isinstance(vm.ctx.exceptions.key_error) {
                        vm.new_attribute_error(format!(
                            "'{}' object has no attribute '{}'",
                            self.class().name(),
                            attr_name.as_str(),
                        ))
                    } else {
                        e
                    }
                })?;
            }
            Ok(())
        } else {
            Err(vm.new_attribute_error(format!(
                "'{}' object has no attribute '{}'",
                self.class().name(),
                attr_name.as_str(),
            )))
        }
    }

    pub fn generic_getattr(&self, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        self.generic_getattr_opt(name.clone(), None, vm)?
            .ok_or_else(|| {
                vm.new_attribute_error(format!(
                    "'{}' object has no attribute '{}'",
                    self.class().name(),
                    name
                ))
            })
    }

    /// CPython _PyObject_GenericGetAttrWithDict
    pub fn generic_getattr_opt(
        &self,
        name_str: PyStrRef,
        dict: Option<PyDictRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        let name = name_str.as_str();
        let obj_cls = self.class();
        let cls_attr_name = vm.ctx.interned_str(&*name_str);
        let cls_attr = match cls_attr_name.and_then(|name| obj_cls.get_attr(name)) {
            Some(descr) => {
                let descr_cls = descr.class();
                let descr_get = descr_cls.mro_find_map(|cls| cls.slots.descr_get.load());
                if let Some(descr_get) = descr_get {
                    if descr_cls
                        .mro_find_map(|cls| cls.slots.descr_set.load())
                        .is_some()
                    {
                        let cls = obj_cls.to_owned().into();
                        return descr_get(descr, Some(self.to_owned()), Some(cls), vm).map(Some);
                    }
                }
                Some((descr, descr_get))
            }
            None => None,
        };

        let dict = dict.or_else(|| self.dict());

        let attr = if let Some(dict) = dict {
            dict.get_item_opt(name, vm)?
        } else {
            None
        };

        if let Some(obj_attr) = attr {
            Ok(Some(obj_attr))
        } else if let Some((attr, descr_get)) = cls_attr {
            match descr_get {
                Some(descr_get) => {
                    let cls = obj_cls.to_owned().into();
                    descr_get(attr, Some(self.to_owned()), Some(cls), vm).map(Some)
                }
                None => Ok(Some(attr)),
            }
        } else {
            Ok(None)
        }
    }

    pub fn del_attr(&self, attr_name: impl IntoPyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        let attr_name = attr_name.into_pystr_ref(vm);
        self.call_set_attr(vm, attr_name, PySetterValue::Delete)
    }

    // Perform a comparison, raising TypeError when the requested comparison
    // operator is not supported.
    // see: CPython PyObject_RichCompare
    #[inline] // called by ExecutingFrame::execute_compare with const op
    fn _cmp(
        &self,
        other: &Self,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<Either<PyObjectRef, bool>> {
        let swapped = op.swapped();
        let call_cmp = |obj: &PyObject, other: &PyObject, op| {
            let cmp = obj
                .class()
                .mro_find_map(|cls| cls.slots.richcompare.load())
                .unwrap();
            let r = match cmp(obj, other, op, vm)? {
                Either::A(obj) => PyArithmeticValue::from_object(vm, obj).map(Either::A),
                Either::B(arithmetic) => arithmetic.map(Either::B),
            };
            Ok(r)
        };

        let mut checked_reverse_op = false;
        let is_strict_subclass = {
            let self_class = self.class();
            let other_class = other.class();
            !self_class.is(other_class) && other_class.fast_issubclass(self_class)
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
    #[inline(always)]
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
        vm.with_recursion("while getting the repr of an object", || {
            let repr = vm.call_special_method(self.to_owned(), identifier!(vm, __repr__), ())?;
            repr.try_into_value(vm)
        })
    }

    pub fn ascii(&self, vm: &VirtualMachine) -> PyResult<ascii::AsciiString> {
        let repr = self.repr(vm)?;
        let ascii = to_ascii(repr.as_str());
        Ok(ascii)
    }

    // Container of the virtual machine state:
    pub fn str(&self, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        if self.class().is(vm.ctx.types.str_type) {
            Ok(self.to_owned().downcast().unwrap())
        } else {
            let s = vm.call_special_method(self.to_owned(), identifier!(vm, __str__), ())?;
            s.try_into_value(vm)
        }
    }

    // Equivalent to check_class. Masks Attribute errors (into TypeErrors) and lets everything
    // else go through.
    fn check_cls<F>(&self, cls: &PyObject, vm: &VirtualMachine, msg: F) -> PyResult
    where
        F: Fn() -> String,
    {
        cls.to_owned()
            .get_attr(identifier!(vm, __bases__), vm)
            .map_err(|e| {
                // Only mask AttributeErrors.
                if e.class().is(vm.ctx.exceptions.attribute_error) {
                    vm.new_type_error(msg())
                } else {
                    e
                }
            })
    }

    fn abstract_issubclass(&self, cls: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        let mut derived = self;
        let mut first_item: PyObjectRef;
        loop {
            if derived.is(cls) {
                return Ok(true);
            }

            let bases = derived
                .to_owned()
                .get_attr(identifier!(vm, __bases__), vm)?;
            let tuple = PyTupleRef::try_from_object(vm, bases)?;

            let n = tuple.len();
            match n {
                0 => {
                    return Ok(false);
                }
                1 => {
                    first_item = tuple.fast_getitem(0).clone();
                    derived = &first_item;
                    continue;
                }
                _ => {
                    if let Some(i) = (0..n).next() {
                        let check = vm.with_recursion("in abstract_issubclass", || {
                            tuple.fast_getitem(i).abstract_issubclass(cls, vm)
                        })?;
                        if check {
                            return Ok(true);
                        }
                    }
                }
            }

            return Ok(false);
        }
    }

    fn recursive_issubclass(&self, cls: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        if let (Ok(obj), Ok(cls)) = (
            PyTypeRef::try_from_object(vm, self.to_owned()),
            PyTypeRef::try_from_object(vm, cls.to_owned()),
        ) {
            Ok(obj.fast_issubclass(&cls))
        } else {
            self.check_cls(self, vm, || {
                format!("issubclass() arg 1 must be a class, not {}", self.class())
            })
            .and(self.check_cls(cls, vm, || {
                format!(
                    "issubclass() arg 2 must be a class, a tuple of classes, or a union, not {}",
                    cls.class()
                )
            }))
            .and(self.abstract_issubclass(cls, vm))
        }
    }

    /// Determines if `self` is a subclass of `cls`, either directly, indirectly or virtually
    /// via the __subclasscheck__ magic method.
    pub fn is_subclass(&self, cls: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        if cls.class().is(vm.ctx.types.type_type) {
            if self.is(cls) {
                return Ok(true);
            }
            return self.recursive_issubclass(cls, vm);
        }

        if let Ok(tuple) = PyTupleRef::try_from_object(vm, cls.to_owned()) {
            for typ in &tuple {
                if vm.with_recursion("in __subclasscheck__", || self.is_subclass(typ, vm))? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }

        if let Ok(meth) =
            vm.get_special_method(cls.to_owned(), identifier!(vm, __subclasscheck__))?
        {
            let ret = vm.with_recursion("in __subclasscheck__", || {
                meth.invoke((self.to_owned(),), vm)
            })?;
            return ret.try_to_bool(vm);
        }

        self.recursive_issubclass(cls, vm)
    }

    fn abstract_isinstance(&self, cls: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        if let Ok(typ) = PyTypeRef::try_from_object(vm, cls.to_owned()) {
            if self.class().fast_issubclass(&typ) {
                Ok(true)
            } else if let Ok(icls) = PyTypeRef::try_from_object(
                vm,
                self.to_owned().get_attr(identifier!(vm, __class__), vm)?,
            ) {
                if icls.is(self.class()) {
                    Ok(false)
                } else {
                    Ok(icls.fast_issubclass(&typ))
                }
            } else {
                Ok(false)
            }
        } else {
            self.check_cls(cls, vm, || {
                format!(
                    "isinstance() arg 2 must be a type or tuple of types, not {}",
                    cls.class()
                )
            })
            .and_then(|_| {
                let icls: PyObjectRef = self.to_owned().get_attr(identifier!(vm, __class__), vm)?;
                if vm.is_none(&icls) {
                    Ok(false)
                } else {
                    icls.abstract_issubclass(cls, vm)
                }
            })
        }
    }

    /// Determines if `self` is an instance of `cls`, either directly, indirectly or virtually via
    /// the __instancecheck__ magic method.
    pub fn is_instance(&self, cls: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        // cpython first does an exact check on the type, although documentation doesn't state that
        // https://github.com/python/cpython/blob/a24107b04c1277e3c1105f98aff5bfa3a98b33a0/Objects/abstract.c#L2408
        if self.class().is(cls) {
            return Ok(true);
        }

        if cls.class().is(vm.ctx.types.type_type) {
            return self.abstract_isinstance(cls, vm);
        }

        if let Ok(tuple) = PyTupleRef::try_from_object(vm, cls.to_owned()) {
            for typ in &tuple {
                if vm.with_recursion("in __instancecheck__", || self.is_instance(typ, vm))? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }

        if let Ok(meth) =
            vm.get_special_method(cls.to_owned(), identifier!(vm, __instancecheck__))?
        {
            let ret = vm.with_recursion("in __instancecheck__", || {
                meth.invoke((self.to_owned(),), vm)
            })?;
            return ret.try_to_bool(vm);
        }

        self.abstract_isinstance(cls, vm)
    }

    pub fn hash(&self, vm: &VirtualMachine) -> PyResult<PyHash> {
        let hash = self.get_class_attr(identifier!(vm, __hash__)).unwrap();
        if vm.is_none(&hash) {
            return Err(vm.new_exception_msg(
                vm.ctx.exceptions.type_error.to_owned(),
                format!("unhashable type: '{}'", self.class().name()),
            ));
        }

        let hash = self
            .class()
            .mro_find_map(|cls| cls.slots.hash.load())
            .unwrap();

        hash(self, vm)
    }

    // type protocol
    // PyObject *PyObject_Type(PyObject *o)

    // int PyObject_TypeCheck(PyObject *o, PyTypeObject *type)

    pub fn length_opt(&self, vm: &VirtualMachine) -> Option<PyResult<usize>> {
        self.to_sequence(vm)
            .length_opt(vm)
            .or_else(|| self.to_mapping().length_opt(vm))
    }

    pub fn length(&self, vm: &VirtualMachine) -> PyResult<usize> {
        self.length_opt(vm).ok_or_else(|| {
            vm.new_type_error(format!(
                "object of type '{}' has no len()",
                self.class().name()
            ))
        })?
    }

    pub fn get_item<K: DictKey + ?Sized>(&self, needle: &K, vm: &VirtualMachine) -> PyResult {
        if let Some(dict) = self.downcast_ref_if_exact::<PyDict>(vm) {
            return dict.get_item(needle, vm);
        }

        let needle = needle.to_pyobject(vm);

        if let Ok(mapping) = PyMapping::try_protocol(self, vm) {
            mapping.subscript(&needle, vm)
        } else if let Ok(seq) = PySequence::try_protocol(self, vm) {
            let i = needle.key_as_isize(vm)?;
            seq.get_item(i, vm)
        } else {
            if self.class().fast_issubclass(vm.ctx.types.type_type) {
                if self.is(vm.ctx.types.type_type) {
                    return PyGenericAlias::new(self.class().to_owned(), needle, vm)
                        .to_pyresult(vm);
                }

                if let Some(class_getitem) =
                    vm.get_attribute_opt(self.to_owned(), identifier!(vm, __class_getitem__))?
                {
                    return vm.invoke(&class_getitem, (needle,));
                }
            }
            Err(vm.new_type_error(format!("'{}' object is not subscriptable", self.class())))
        }
    }

    pub fn set_item<K: DictKey + ?Sized>(
        &self,
        needle: &K,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let Some(dict) = self.downcast_ref_if_exact::<PyDict>(vm) {
            return dict.set_item(needle, value, vm);
        }

        let mapping = self.to_mapping();
        if let Some(f) = mapping.methods.ass_subscript.load() {
            let needle = needle.to_pyobject(vm);
            return f(mapping, &needle, Some(value), vm);
        }

        let seq = self.to_sequence(vm);
        if let Some(f) = seq.methods.ass_item.load() {
            let i = needle.key_as_isize(vm)?;
            return f(seq, i, Some(value), vm);
        }

        Err(vm.new_type_error(format!(
            "'{}' does not support item assignment",
            self.class()
        )))
    }

    pub fn del_item<K: DictKey + ?Sized>(&self, needle: &K, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(dict) = self.downcast_ref_if_exact::<PyDict>(vm) {
            return dict.del_item(needle, vm);
        }

        let mapping = self.to_mapping();
        if let Some(f) = mapping.methods.ass_subscript.load() {
            let needle = needle.to_pyobject(vm);
            return f(mapping, &needle, None, vm);
        }
        let seq = self.to_sequence(vm);
        if let Some(f) = seq.methods.ass_item.load() {
            let i = needle.key_as_isize(vm)?;
            return f(seq, i, None, vm);
        }

        Err(vm.new_type_error(format!("'{}' does not support item deletion", self.class())))
    }
}
