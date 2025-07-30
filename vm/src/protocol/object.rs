//! Object Protocol
//! <https://docs.python.org/3/c-api/object.html>

use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyRef, PyResult, TryFromObject, VirtualMachine,
    builtins::{
        PyAsyncGen, PyBytes, PyDict, PyDictRef, PyGenericAlias, PyInt, PyList, PyStr, PyTuple,
        PyTupleRef, PyType, PyTypeRef, PyUtf8Str, pystr::AsPyStr,
    },
    bytes_inner::ByteInnerNewOptions,
    common::{hash::PyHash, str::to_ascii},
    convert::{ToPyObject, ToPyResult},
    dict_inner::DictKey,
    function::{Either, OptionalArg, PyArithmeticValue, PySetterValue},
    object::PyPayload,
    protocol::{PyIter, PyMapping, PySequence},
    types::{Constructor, PyComparisonOp},
};

// RustPython doesn't need these items
// PyObject *Py_NotImplemented
// Py_RETURN_NOTIMPLEMENTED

impl PyObjectRef {
    // int PyObject_Print(PyObject *o, FILE *fp, int flags)

    // PyObject *PyObject_GenericGetDict(PyObject *o, void *context)
    // int PyObject_GenericSetDict(PyObject *o, PyObject *value, void *context)

    #[inline(always)]
    pub fn rich_compare(self, other: Self, op_id: PyComparisonOp, vm: &VirtualMachine) -> PyResult {
        self._cmp(&other, op_id, vm).map(|res| res.to_pyobject(vm))
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
    pub fn dir(self, vm: &VirtualMachine) -> PyResult<PyList> {
        let attributes = self.class().get_attributes();

        let dict = PyDict::from_attributes(attributes, vm)?.into_ref(&vm.ctx);

        if let Some(object_dict) = self.dict() {
            vm.call_method(
                dict.as_object(),
                identifier!(vm, update).as_str(),
                (object_dict,),
            )?;
        }

        let attributes: Vec<_> = dict.into_iter().map(|(k, _v)| k).collect();

        Ok(PyList::from(attributes))
    }
}

impl PyObject {
    /// Takes an object and returns an iterator for it.
    /// This is typically a new iterator but if the argument is an iterator, this
    /// returns itself.
    pub fn get_iter(&self, vm: &VirtualMachine) -> PyResult<PyIter> {
        // PyObject_GetIter
        PyIter::try_from_object(vm, self.to_owned())
    }

    // PyObject *PyObject_GetAIter(PyObject *o)
    pub fn get_aiter(&self, vm: &VirtualMachine) -> PyResult {
        if self.downcastable::<PyAsyncGen>() {
            vm.call_special_method(self, identifier!(vm, __aiter__), ())
        } else {
            Err(vm.new_type_error("wrong argument type"))
        }
    }

    pub fn has_attr<'a>(&self, attr_name: impl AsPyStr<'a>, vm: &VirtualMachine) -> PyResult<bool> {
        self.get_attr(attr_name, vm).map(|o| !vm.is_none(&o))
    }

    /// Get an attribute by name.
    /// `attr_name` can be a `&str`, `String`, or `PyStrRef`.
    pub fn get_attr<'a>(&self, attr_name: impl AsPyStr<'a>, vm: &VirtualMachine) -> PyResult {
        let attr_name = attr_name.as_pystr(&vm.ctx);
        self.get_attr_inner(attr_name, vm)
    }

    // get_attribute should be used for full attribute access (usually from user code).
    #[cfg_attr(feature = "flame-it", flame("PyObjectRef"))]
    #[inline]
    pub(crate) fn get_attr_inner(&self, attr_name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        vm_trace!("object.__getattribute__: {:?} {:?}", self, attr_name);
        let getattro = self
            .class()
            .mro_find_map(|cls| cls.slots.getattro.load())
            .unwrap();
        getattro(self, attr_name, vm).inspect_err(|exc| {
            vm.set_attribute_error_context(exc, self.to_owned(), attr_name.to_owned());
        })
    }

    pub fn call_set_attr(
        &self,
        vm: &VirtualMachine,
        attr_name: &Py<PyStr>,
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

    pub fn set_attr<'a>(
        &self,
        attr_name: impl AsPyStr<'a>,
        attr_value: impl Into<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let attr_name = attr_name.as_pystr(&vm.ctx);
        let attr_value = attr_value.into();
        self.call_set_attr(vm, attr_name, PySetterValue::Assign(attr_value))
    }

    // int PyObject_GenericSetAttr(PyObject *o, PyObject *name, PyObject *value)
    #[cfg_attr(feature = "flame-it", flame)]
    pub fn generic_setattr(
        &self,
        attr_name: &Py<PyStr>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        vm_trace!("object.__setattr__({:?}, {}, {:?})", self, attr_name, value);
        if let Some(attr) = vm
            .ctx
            .interned_str(attr_name)
            .and_then(|attr_name| self.get_class_attr(attr_name))
        {
            let descr_set = attr.class().mro_find_map(|cls| cls.slots.descr_set.load());
            if let Some(descriptor) = descr_set {
                return descriptor(&attr, self.to_owned(), value, vm);
            }
        }

        if let Some(dict) = self.dict() {
            if let PySetterValue::Assign(value) = value {
                dict.set_item(attr_name, value, vm)?;
            } else {
                dict.del_item(attr_name, vm).map_err(|e| {
                    if e.fast_isinstance(vm.ctx.exceptions.key_error) {
                        vm.new_no_attribute_error(self.to_owned(), attr_name.to_owned())
                    } else {
                        e
                    }
                })?;
            }
            Ok(())
        } else {
            Err(vm.new_no_attribute_error(self.to_owned(), attr_name.to_owned()))
        }
    }

    pub fn generic_getattr(&self, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        self.generic_getattr_opt(name, None, vm)?
            .ok_or_else(|| vm.new_no_attribute_error(self.to_owned(), name.to_owned()))
    }

    /// CPython _PyObject_GenericGetAttrWithDict
    pub fn generic_getattr_opt(
        &self,
        name_str: &Py<PyStr>,
        dict: Option<PyDictRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        let name = name_str.as_wtf8();
        let obj_cls = self.class();
        let cls_attr_name = vm.ctx.interned_str(name_str);
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

    pub fn del_attr<'a>(&self, attr_name: impl AsPyStr<'a>, vm: &VirtualMachine) -> PyResult<()> {
        let attr_name = attr_name.as_pystr(&vm.ctx);
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
        let call_cmp = |obj: &Self, other: &Self, op| {
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
            _ => Err(vm.new_unsupported_bin_op_error(self, other, op.operator_token())),
        }
    }
    #[inline(always)]
    pub fn rich_compare_bool(
        &self,
        other: &Self,
        op_id: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        match self._cmp(other, op_id, vm)? {
            Either::A(obj) => obj.try_to_bool(vm),
            Either::B(other) => Ok(other),
        }
    }

    pub fn repr_utf8(&self, vm: &VirtualMachine) -> PyResult<PyRef<PyUtf8Str>> {
        self.repr(vm)?.try_into_utf8(vm)
    }

    pub fn repr(&self, vm: &VirtualMachine) -> PyResult<PyRef<PyStr>> {
        vm.with_recursion("while getting the repr of an object", || {
            // TODO: RustPython does not implement type slots inheritance yet
            self.class()
                .mro_find_map(|cls| cls.slots.repr.load())
                .map_or_else(
                    || {
                        Err(vm.new_runtime_error(format!(
                    "BUG: object of type '{}' has no __repr__ method. This is a bug in RustPython.",
                    self.class().name()
                )))
                    },
                    |repr| repr(self, vm),
                )
        })
    }

    pub fn ascii(&self, vm: &VirtualMachine) -> PyResult<ascii::AsciiString> {
        let repr = self.repr_utf8(vm)?;
        let ascii = to_ascii(repr.as_str());
        Ok(ascii)
    }

    pub fn str_utf8(&self, vm: &VirtualMachine) -> PyResult<PyRef<PyUtf8Str>> {
        self.str(vm)?.try_into_utf8(vm)
    }
    pub fn str(&self, vm: &VirtualMachine) -> PyResult<PyRef<PyStr>> {
        let obj = match self.to_owned().downcast_exact::<PyStr>(vm) {
            Ok(s) => return Ok(s.into_pyref()),
            Err(obj) => obj,
        };
        // TODO: replace to obj.class().slots.str
        let str_method = match vm.get_special_method(&obj, identifier!(vm, __str__))? {
            Some(str_method) => str_method,
            None => return obj.repr(vm),
        };
        let s = str_method.invoke((), vm)?;
        s.downcast::<PyStr>().map_err(|obj| {
            vm.new_type_error(format!(
                "__str__ returned non-string (type {})",
                obj.class().name()
            ))
        })
    }

    // Equivalent to CPython's check_class. Returns Ok(()) if cls is a valid class,
    // Err with TypeError if not. Uses abstract_get_bases internally.
    fn check_class<F>(&self, vm: &VirtualMachine, msg: F) -> PyResult<()>
    where
        F: Fn() -> String,
    {
        let cls = self;
        match cls.abstract_get_bases(vm)? {
            Some(_bases) => Ok(()), // Has __bases__, it's a valid class
            None => {
                // No __bases__ or __bases__ is not a tuple
                Err(vm.new_type_error(msg()))
            }
        }
    }

    /// abstract_get_bases() has logically 4 return states:
    /// 1. getattr(cls, '__bases__') could raise an AttributeError
    /// 2. getattr(cls, '__bases__') could raise some other exception
    /// 3. getattr(cls, '__bases__') could return a tuple
    /// 4. getattr(cls, '__bases__') could return something other than a tuple
    ///
    /// Only state #3 returns Some(tuple). AttributeErrors are masked by returning None.
    /// If an object other than a tuple comes out of __bases__, then again, None is returned.
    /// Other exceptions are propagated.
    fn abstract_get_bases(&self, vm: &VirtualMachine) -> PyResult<Option<PyTupleRef>> {
        match vm.get_attribute_opt(self.to_owned(), identifier!(vm, __bases__))? {
            Some(bases) => {
                // Check if it's a tuple
                match PyTupleRef::try_from_object(vm, bases) {
                    Ok(tuple) => Ok(Some(tuple)),
                    Err(_) => Ok(None), // Not a tuple, return None
                }
            }
            None => Ok(None), // AttributeError was masked
        }
    }

    fn abstract_issubclass(&self, cls: &Self, vm: &VirtualMachine) -> PyResult<bool> {
        // Store the current derived class to check
        let mut bases: PyTupleRef;
        let mut derived = self;

        // First loop: handle single inheritance without recursion
        let bases = loop {
            if derived.is(cls) {
                return Ok(true);
            }

            let Some(derived_bases) = derived.abstract_get_bases(vm)? else {
                return Ok(false);
            };

            let n = derived_bases.len();
            match n {
                0 => return Ok(false),
                1 => {
                    // Avoid recursion in the single inheritance case
                    // Get the next derived class and continue the loop
                    bases = derived_bases;
                    derived = &bases.as_slice()[0];
                    continue;
                }
                _ => {
                    // Multiple inheritance - handle recursively
                    break derived_bases;
                }
            }
        };

        let n = bases.len();
        // At this point we know n >= 2
        debug_assert!(n >= 2);

        for i in 0..n {
            let result = vm.with_recursion("in __issubclass__", || {
                bases.as_slice()[i].abstract_issubclass(cls, vm)
            })?;
            if result {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn recursive_issubclass(&self, cls: &Self, vm: &VirtualMachine) -> PyResult<bool> {
        // Fast path for both being types (matches CPython's PyType_Check)
        if let Some(cls) = PyType::check(cls)
            && let Some(derived) = PyType::check(self)
        {
            // PyType_IsSubtype equivalent
            return Ok(derived.is_subtype(cls));
        }
        // Check if derived is a class
        self.check_class(vm, || {
            format!("issubclass() arg 1 must be a class, not {}", self.class())
        })?;

        // Check if cls is a class, tuple, or union (matches CPython's order and message)
        if !cls.class().is(vm.ctx.types.union_type) {
            cls.check_class(vm, || {
                format!(
                    "issubclass() arg 2 must be a class, a tuple of classes, or a union, not {}",
                    cls.class()
                )
            })?;
        }

        self.abstract_issubclass(cls, vm)
    }

    /// Real issubclass check without going through __subclasscheck__
    /// This is equivalent to CPython's _PyObject_RealIsSubclass which just calls recursive_issubclass
    pub fn real_is_subclass(&self, cls: &Self, vm: &VirtualMachine) -> PyResult<bool> {
        self.recursive_issubclass(cls, vm)
    }

    /// Determines if `self` is a subclass of `cls`, either directly, indirectly or virtually
    /// via the __subclasscheck__ magic method.
    /// PyObject_IsSubclass/object_issubclass
    pub fn is_subclass(&self, cls: &Self, vm: &VirtualMachine) -> PyResult<bool> {
        let derived = self;
        // PyType_CheckExact(cls)
        if cls.class().is(vm.ctx.types.type_type) {
            if derived.is(cls) {
                return Ok(true);
            }
            return derived.recursive_issubclass(cls, vm);
        }

        // Check for Union type - CPython handles this before tuple
        let cls = if cls.class().is(vm.ctx.types.union_type) {
            // Get the __args__ attribute which contains the union members
            // Match CPython's _Py_union_args which directly accesses the args field
            let union = cls
                .downcast_ref::<crate::builtins::PyUnion>()
                .expect("union is already checked");
            union.args().as_object()
        } else {
            cls
        };

        // Check if cls is a tuple
        if let Some(tuple) = cls.downcast_ref::<PyTuple>() {
            for item in tuple {
                if vm.with_recursion("in __subclasscheck__", || derived.is_subclass(item, vm))? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }

        // Check for __subclasscheck__ method using lookup_special (matches CPython)
        if let Some(checker) = cls.lookup_special(identifier!(vm, __subclasscheck__), vm) {
            let res = vm.with_recursion("in __subclasscheck__", || {
                checker.call((derived.to_owned(),), vm)
            })?;
            return res.try_to_bool(vm);
        }

        derived.recursive_issubclass(cls, vm)
    }

    // _PyObject_RealIsInstance
    pub(crate) fn real_is_instance(&self, cls: &Self, vm: &VirtualMachine) -> PyResult<bool> {
        self.object_isinstance(cls, vm)
    }

    /// Real isinstance check without going through __instancecheck__
    /// This is equivalent to CPython's _PyObject_RealIsInstance/object_isinstance
    fn object_isinstance(&self, cls: &Self, vm: &VirtualMachine) -> PyResult<bool> {
        if let Ok(cls) = cls.try_to_ref::<PyType>(vm) {
            // PyType_Check(cls) - cls is a type object
            let mut retval = self.class().is_subtype(cls);
            if !retval {
                // Check __class__ attribute, only masking AttributeError
                if let Some(i_cls) =
                    vm.get_attribute_opt(self.to_owned(), identifier!(vm, __class__))?
                {
                    if let Ok(i_cls_type) = PyTypeRef::try_from_object(vm, i_cls) {
                        if !i_cls_type.is(self.class()) {
                            retval = i_cls_type.is_subtype(cls);
                        }
                    }
                }
            }
            Ok(retval)
        } else {
            // Not a type object, check if it's a valid class
            cls.check_class(vm, || {
                format!(
                    "isinstance() arg 2 must be a type, a tuple of types, or a union, not {}",
                    cls.class()
                )
            })?;

            // Get __class__ attribute and check, only masking AttributeError
            if let Some(i_cls) =
                vm.get_attribute_opt(self.to_owned(), identifier!(vm, __class__))?
            {
                i_cls.abstract_issubclass(cls, vm)
            } else {
                Ok(false)
            }
        }
    }

    /// Determines if `self` is an instance of `cls`, either directly, indirectly or virtually via
    /// the __instancecheck__ magic method.
    pub fn is_instance(&self, cls: &Self, vm: &VirtualMachine) -> PyResult<bool> {
        self.object_recursive_isinstance(cls, vm)
    }

    // This is object_recursive_isinstance from CPython's Objects/abstract.c
    fn object_recursive_isinstance(&self, cls: &Self, vm: &VirtualMachine) -> PyResult<bool> {
        // PyObject_TypeCheck(inst, (PyTypeObject *)cls)
        // This is an exact check of the type
        if self.class().is(cls) {
            return Ok(true);
        }

        // PyType_CheckExact(cls) optimization
        if cls.class().is(vm.ctx.types.type_type) {
            // When cls is exactly a type (not a subclass), use object_isinstance
            // to avoid going through __instancecheck__ (matches CPython behavior)
            return self.object_isinstance(cls, vm);
        }

        // Check for Union type (e.g., int | str) - CPython checks this before tuple
        let cls = if cls.class().is(vm.ctx.types.union_type) {
            // Match CPython's _Py_union_args which directly accesses the args field
            let union = cls
                .try_to_ref::<crate::builtins::PyUnion>(vm)
                .expect("checked by is");
            union.args().as_object()
        } else {
            cls
        };

        // Check if cls is a tuple
        if let Some(tuple) = cls.downcast_ref::<PyTuple>() {
            for item in tuple {
                if vm.with_recursion("in __instancecheck__", || {
                    self.object_recursive_isinstance(item, vm)
                })? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }

        // Check for __instancecheck__ method using lookup_special (matches CPython)
        if let Some(checker) = cls.lookup_special(identifier!(vm, __instancecheck__), vm) {
            let res = vm.with_recursion("in __instancecheck__", || {
                checker.call((self.to_owned(),), vm)
            })?;
            return res.try_to_bool(vm);
        }

        // Fall back to object_isinstance (without going through __instancecheck__ again)
        self.object_isinstance(cls, vm)
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
    pub fn obj_type(&self) -> PyObjectRef {
        self.class().to_owned().into()
    }

    // int PyObject_TypeCheck(PyObject *o, PyTypeObject *type)
    pub fn type_check(&self, typ: &Py<PyType>) -> bool {
        self.fast_isinstance(typ)
    }

    pub fn length_opt(&self, vm: &VirtualMachine) -> Option<PyResult<usize>> {
        self.to_sequence()
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
                    return PyGenericAlias::from_args(self.class().to_owned(), needle, vm)
                        .to_pyresult(vm);
                }

                if let Some(class_getitem) =
                    vm.get_attribute_opt(self.to_owned(), identifier!(vm, __class_getitem__))?
                {
                    return class_getitem.call((needle,), vm);
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

        let seq = self.to_sequence();
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
        let seq = self.to_sequence();
        if let Some(f) = seq.methods.ass_item.load() {
            let i = needle.key_as_isize(vm)?;
            return f(seq, i, None, vm);
        }

        Err(vm.new_type_error(format!("'{}' does not support item deletion", self.class())))
    }

    /// Equivalent to CPython's _PyObject_LookupSpecial
    /// Looks up a special method in the type's MRO without checking instance dict.
    /// Returns None if not found (masking AttributeError like CPython).
    pub fn lookup_special(&self, attr: &Py<PyStr>, vm: &VirtualMachine) -> Option<PyObjectRef> {
        let obj_cls = self.class();

        // Use PyType::lookup_ref (equivalent to CPython's _PyType_LookupRef)
        let res = obj_cls.lookup_ref(attr, vm)?;

        // If it's a descriptor, call its __get__ method
        let descr_get = res.class().mro_find_map(|cls| cls.slots.descr_get.load());
        if let Some(descr_get) = descr_get {
            let obj_cls = obj_cls.to_owned().into();
            // CPython ignores exceptions in _PyObject_LookupSpecial and returns NULL
            descr_get(res, Some(self.to_owned()), Some(obj_cls), vm).ok()
        } else {
            Some(res)
        }
    }
}
