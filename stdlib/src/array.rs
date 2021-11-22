pub(crate) use array::make_module;

#[pymodule(name = "array")]
mod array {
    use crate::common::{
        atomic::{self, AtomicUsize},
        lock::{
            PyMappedRwLockReadGuard, PyMappedRwLockWriteGuard, PyRwLock, PyRwLockReadGuard,
            PyRwLockWriteGuard,
        },
        str::wchar_t,
    };
    use crate::vm::{
        builtins::{
            PyByteArray, PyBytes, PyBytesRef, PyDictRef, PyFloat, PyInt, PyIntRef, PyList,
            PyListRef, PyStr, PyStrRef, PyTupleRef, PyTypeRef,
        },
        class_or_notimplemented,
        function::{
            ArgBytesLike, ArgIntoFloat, ArgIterable, IntoPyObject, IntoPyResult, OptionalArg,
        },
        protocol::{
            BufferDescriptor, BufferMethods, BufferResizeGuard, PyBuffer, PyIterReturn,
            PyMappingMethods,
        },
        sequence::{SequenceMutOp, SequenceOp},
        sliceable::{PySliceableSequence, PySliceableSequenceMut, SaturatedSlice, SequenceIndex},
        types::{
            AsBuffer, AsMapping, Comparable, Constructor, IterNext, IterNextIterable, Iterable,
            PyComparisonOp,
        },
        IdProtocol, PyComparisonValue, PyObject, PyObjectRef, PyObjectView, PyObjectWrap, PyRef,
        PyResult, PyValue, TryFromObject, TypeProtocol, VirtualMachine,
    };
    use itertools::Itertools;
    use num_traits::ToPrimitive;
    use std::cmp::Ordering;
    use std::{fmt, os::raw};

    macro_rules! def_array_enum {
        ($(($n:ident, $t:ty, $c:literal, $scode:literal)),*$(,)?) => {
            #[derive(Debug, Clone)]
            pub enum ArrayContentType {
                $($n(Vec<$t>),)*
            }

            #[allow(clippy::naive_bytecount, clippy::float_cmp)]
            impl ArrayContentType {
                fn from_char(c: char) -> Result<Self, String> {
                    match c {
                        $($c => Ok(ArrayContentType::$n(Vec::new())),)*
                        _ => Err(
                            "bad typecode (must be b, B, u, h, H, i, I, l, L, q, Q, f or d)".into()
                        ),
                    }
                }

                fn typecode(&self) -> char {
                    match self {
                        $(ArrayContentType::$n(_) => $c,)*
                    }
                }

                fn typecode_str(&self) -> &'static str {
                    match self {
                        $(ArrayContentType::$n(_) => $scode,)*
                    }
                }

                fn itemsize_of_typecode(c: char) -> Option<usize> {
                    match c {
                        $($c => Some(std::mem::size_of::<$t>()),)*
                        _ => None,
                    }
                }

                fn itemsize(&self) -> usize {
                    match self {
                        $(ArrayContentType::$n(_) => std::mem::size_of::<$t>(),)*
                    }
                }

                fn addr(&self) -> usize {
                    match self {
                        $(ArrayContentType::$n(v) => v.as_ptr() as usize,)*
                    }
                }

                fn len(&self) -> usize {
                    match self {
                        $(ArrayContentType::$n(v) => v.len(),)*
                    }
                }

                fn reserve(&mut self, len: usize) {
                    match self {
                        $(ArrayContentType::$n(v) => v.reserve(len),)*
                    }
                }

                fn push(&mut self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            let val = <$t>::try_into_from_object(vm, obj)?;
                            v.push(val);
                        })*
                    }
                    Ok(())
                }

                fn pop(&mut self, i: isize, vm: &VirtualMachine) -> PyResult {
                    match self {
                        $(ArrayContentType::$n(v) => {
                        let i = v.wrap_index(i).ok_or_else(|| {
                            vm.new_index_error("pop index out of range".to_owned())
                        })?;
                            v.remove(i).into_pyresult(vm)
                        })*
                    }
                }

                fn insert(
                    &mut self,
                    i: isize,
                    obj: PyObjectRef,
                    vm: &VirtualMachine
                ) -> PyResult<()> {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            let val = <$t>::try_into_from_object(vm, obj)?;
                            v.insert(v.saturate_index(i), val);
                        })*
                    }
                    Ok(())
                }

                fn count(&self, obj: PyObjectRef, vm: &VirtualMachine) -> usize {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            if let Ok(val) = <$t>::try_into_from_object(vm, obj) {
                                v.iter().filter(|&&a| a == val).count()
                            } else {
                                0
                            }
                        })*
                    }
                }

                fn remove(&mut self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()>{
                    match self {
                        $(ArrayContentType::$n(v) => {
                            if let Ok(val) = <$t>::try_into_from_object(vm, obj) {
                                if let Some(pos) = v.iter().position(|&a| a == val) {
                                    v.remove(pos);
                                    return Ok(());
                                }
                            }
                            Err(vm.new_value_error("array.remove(x): x not in array".to_owned()))
                        })*
                    }
                }

                fn frombytes_move(&mut self, b: Vec<u8>) {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            if v.is_empty() {
                                // safe because every configuration of bytes for the types we
                                // support are valid
                                let b = std::mem::ManuallyDrop::new(b);
                                let ptr = b.as_ptr() as *mut $t;
                                let len = b.len() / std::mem::size_of::<$t>();
                                let capacity = b.capacity() / std::mem::size_of::<$t>();
                                *v = unsafe { Vec::from_raw_parts(ptr, len, capacity) };
                            } else {
                                self.frombytes(&b);
                            }
                        })*
                    }
                }

                fn frombytes(&mut self, b: &[u8]) {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            // safe because every configuration of bytes for the types we
                            // support are valid
                            let ptr = b.as_ptr() as *const $t;
                            let ptr_len = b.len() / std::mem::size_of::<$t>();
                            let slice = unsafe { std::slice::from_raw_parts(ptr, ptr_len) };
                            v.extend_from_slice(slice);
                        })*
                    }
                }

                fn fromlist(&mut self, list: &PyList, vm: &VirtualMachine) -> PyResult<()> {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            // convert list before modify self
                            let mut list: Vec<$t> = list
                                .borrow_vec()
                                .iter()
                                .cloned()
                                .map(|value| <$t>::try_into_from_object(vm, value))
                                .try_collect()?;
                            v.append(&mut list);
                            Ok(())
                        })*
                    }
                }

                fn get_bytes(&self) -> &[u8] {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            // safe because we're just reading memory as bytes
                            let ptr = v.as_ptr() as *const u8;
                            let ptr_len = v.len() * std::mem::size_of::<$t>();
                            unsafe { std::slice::from_raw_parts(ptr, ptr_len) }
                        })*
                    }
                }

                fn get_bytes_mut(&mut self) -> &mut [u8] {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            // safe because we're just reading memory as bytes
                            let ptr = v.as_ptr() as *mut u8;
                            let ptr_len = v.len() * std::mem::size_of::<$t>();
                            unsafe { std::slice::from_raw_parts_mut(ptr, ptr_len) }
                        })*
                    }
                }

                fn index(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            if let Ok(val) = <$t>::try_into_from_object(vm, obj) {
                                if let Some(pos) = v.iter().position(|&a| a == val) {
                                    return Ok(pos);
                                }
                            }
                            Err(vm.new_value_error("array.index(x): x not in array".to_owned()))
                        })*
                    }
                }

                fn reverse(&mut self) {
                    match self {
                        $(ArrayContentType::$n(v) => v.reverse(),)*
                    }
                }

                fn getitem_by_idx(
                    &self,
                    i: usize,
                    vm: &VirtualMachine
                ) -> PyResult<Option<PyObjectRef>> {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            v.get(i).map(|x| x.into_pyresult(vm)).transpose()
                        })*
                    }
                }

                fn getitem(
                    &self,
                    needle: PyObjectRef,
                    vm: &VirtualMachine
                ) -> PyResult {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            match SequenceIndex::try_from_object_for(vm, needle, "array")? {
                                SequenceIndex::Int(i) => {
                                    let pos_index = v.wrap_index(i).ok_or_else(|| {
                                        vm.new_index_error("array index out of range".to_owned())
                                    })?;
                                    v.get(pos_index).unwrap().into_pyresult(vm)
                                }
                                SequenceIndex::Slice(slice) => {
                                    // TODO: Use interface similar to set/del item. This can
                                    // still hang.
                                    let slice = slice.to_saturated(vm)?;
                                    let elements = v.get_slice_items(vm, slice)?;
                                    let array: PyArray = ArrayContentType::$n(elements).into();
                                    Ok(array.into_object(vm))
                                }
                            }
                        })*
                    }
                }

                fn setitem_by_slice(
                    &mut self,
                    slice: SaturatedSlice,
                    items: &ArrayContentType,
                    vm: &VirtualMachine
                ) -> PyResult<()> {
                    match self {
                        $(Self::$n(elements) => if let ArrayContentType::$n(items) = items {
                            elements.set_slice_items(vm, slice, items)
                        } else {
                            Err(vm.new_type_error(
                                "bad argument type for built-in operation".to_owned()
                            ))
                        },)*
                    }
                }

                fn setitem_by_slice_no_resize(
                    &mut self,
                    slice: SaturatedSlice,
                    items: &ArrayContentType,
                    vm: &VirtualMachine
                ) -> PyResult<()> {
                    match self {
                        $(Self::$n(elements) => if let ArrayContentType::$n(items) = items {
                            elements.set_slice_items_no_resize(vm, slice, items)
                        } else {
                            Err(vm.new_type_error(
                                "bad argument type for built-in operation".to_owned()
                            ))
                        },)*
                    }
                }

                fn setitem_by_idx(
                    &mut self,
                    i: isize,
                    value: PyObjectRef,
                    vm: &VirtualMachine
                ) -> PyResult<()> {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            let i = v.wrap_index(i).ok_or_else(|| {
                                vm.new_index_error("array assignment index out of range".to_owned())
                            })?;
                            v[i] = <$t>::try_into_from_object(vm, value)?
                        })*
                    }
                    Ok(())
                }

                fn delitem_by_idx(&mut self, i: isize, vm: &VirtualMachine) -> PyResult<()> {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            let i = v.wrap_index(i).ok_or_else(|| {
                                vm.new_index_error("array assignment index out of range".to_owned())
                            })?;
                            v.remove(i);
                        })*
                    }
                    Ok(())
                }

                fn delitem_by_slice(
                    &mut self,
                    slice: SaturatedSlice,
                    vm: &VirtualMachine
                ) -> PyResult<()> {
                    match self {
                        $(ArrayContentType::$n(elements) => {
                            elements.delete_slice(vm, slice)
                        })*
                    }
                }

                fn add(&self, other: &ArrayContentType, vm: &VirtualMachine) -> PyResult<Self> {
                    match self {
                        $(ArrayContentType::$n(v) => if let ArrayContentType::$n(other) = other {
                            let elements = v.iter().chain(other.iter()).cloned().collect();
                            Ok(ArrayContentType::$n(elements))
                        } else {
                            Err(vm.new_type_error(
                                "bad argument type for built-in operation".to_owned()
                            ))
                        },)*
                    }
                }

                fn iadd(&mut self, other: &ArrayContentType, vm: &VirtualMachine) -> PyResult<()> {
                    match self {
                        $(ArrayContentType::$n(v) => if let ArrayContentType::$n(other) = other {
                            v.extend(other);
                            Ok(())
                        } else {
                            Err(vm.new_type_error(
                                "can only extend with array of same kind".to_owned()
                            ))
                        },)*
                    }
                }

                fn mul(&self, value: isize, vm: &VirtualMachine) -> PyResult<Self> {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            // MemoryError instead Overflow Error, hard to says it is right
                            // but it is how cpython doing right now
                            let elements = v.mul(vm, value).map_err(|_| vm.new_memory_error("".to_owned()))?;
                            Ok(ArrayContentType::$n(elements))
                        })*
                    }
                }

                fn imul(&mut self, value: isize, vm: &VirtualMachine) -> PyResult<()> {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            // MemoryError instead Overflow Error, hard to says it is right
                            // but it is how cpython doing right now
                            v.imul(vm, value).map_err(|_| vm.new_memory_error("".to_owned()))
                        })*
                    }
                }

                fn byteswap(&mut self) {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            for element in v.iter_mut() {
                                let x = element.byteswap();
                                *element = x;
                            }
                        })*
                    }
                }

                fn repr(&self, class_name: &str, _vm: &VirtualMachine) -> PyResult<String> {
                    // we don't need ReprGuard here
                    let s = match self {
                        $(ArrayContentType::$n(v) => {
                            if v.is_empty() {
                                format!("{}('{}')", class_name, $c)
                            } else {
                                format!("{}('{}', [{}])", class_name, $c, v.iter().format(", "))
                            }
                        })*
                    };
                    Ok(s)
                }

                fn iter<'a, 'vm: 'a>(
                    &'a self,
                    vm: &'vm VirtualMachine
                ) -> impl Iterator<Item = PyResult> + 'a {
                    (0..self.len()).map(move |i| self.getitem_by_idx(i, vm).map(Option::unwrap))
                }

                fn cmp(&self, other: &ArrayContentType) -> Result<Option<Ordering>, ()> {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            if let ArrayContentType::$n(other) = other {
                                Ok(PartialOrd::partial_cmp(v, other))
                            } else {
                                Err(())
                            }
                        })*
                    }
                }

                fn get_objects(&self, vm: &VirtualMachine) -> Vec<PyObjectRef> {
                    match self {
                        $(ArrayContentType::$n(v) => {
                            v.iter().map(|&x| x.to_object(vm)).collect()
                        })*
                    }
                }
            }
        };
    }

    def_array_enum!(
        (SignedByte, i8, 'b', "b"),
        (UnsignedByte, u8, 'B', "B"),
        (PyUnicode, WideChar, 'u', "u"),
        (SignedShort, raw::c_short, 'h', "h"),
        (UnsignedShort, raw::c_ushort, 'H', "H"),
        (SignedInt, raw::c_int, 'i', "i"),
        (UnsignedInt, raw::c_uint, 'I', "I"),
        (SignedLong, raw::c_long, 'l', "l"),
        (UnsignedLong, raw::c_ulong, 'L', "L"),
        (SignedLongLong, raw::c_longlong, 'q', "q"),
        (UnsignedLongLong, raw::c_ulonglong, 'Q', "Q"),
        (Float, f32, 'f', "f"),
        (Double, f64, 'd', "d"),
    );

    #[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Debug)]
    pub struct WideChar(wchar_t);

    trait ArrayElement: Sized {
        fn try_into_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self>;
        fn byteswap(self) -> Self;
        fn to_object(self, vm: &VirtualMachine) -> PyObjectRef;
    }

    macro_rules! impl_array_element {
        ($(($t:ty, $f_from:path, $f_swap:path, $f_to:path),)*) => {$(
            impl ArrayElement for $t {
                fn try_into_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
                    $f_from(vm, obj)
                }
                fn byteswap(self) -> Self {
                    $f_swap(self)
                }
                fn to_object(self, vm: &VirtualMachine) -> PyObjectRef {
                    $f_to(self).into_object(vm)
                }
            }
        )*};
    }

    impl_array_element!(
        (i8, i8::try_from_object, i8::swap_bytes, PyInt::from),
        (u8, u8::try_from_object, u8::swap_bytes, PyInt::from),
        (i16, i16::try_from_object, i16::swap_bytes, PyInt::from),
        (u16, u16::try_from_object, u16::swap_bytes, PyInt::from),
        (i32, i32::try_from_object, i32::swap_bytes, PyInt::from),
        (u32, u32::try_from_object, u32::swap_bytes, PyInt::from),
        (i64, i64::try_from_object, i64::swap_bytes, PyInt::from),
        (u64, u64::try_from_object, u64::swap_bytes, PyInt::from),
        (
            f32,
            f32_try_into_from_object,
            f32_swap_bytes,
            pyfloat_from_f32
        ),
        (f64, f64_try_into_from_object, f64_swap_bytes, PyFloat::from),
    );

    fn f32_swap_bytes(x: f32) -> f32 {
        f32::from_bits(x.to_bits().swap_bytes())
    }

    fn f64_swap_bytes(x: f64) -> f64 {
        f64::from_bits(x.to_bits().swap_bytes())
    }

    fn f32_try_into_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<f32> {
        ArgIntoFloat::try_from_object(vm, obj).map(|x| x.to_f64() as f32)
    }

    fn f64_try_into_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<f64> {
        ArgIntoFloat::try_from_object(vm, obj).map(|x| x.to_f64())
    }

    fn pyfloat_from_f32(value: f32) -> PyFloat {
        PyFloat::from(value as f64)
    }

    impl ArrayElement for WideChar {
        fn try_into_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            PyStrRef::try_from_object(vm, obj)?
                .as_str()
                .chars()
                .exactly_one()
                .map(|ch| Self(ch as _))
                .map_err(|_| vm.new_type_error("array item must be unicode character".into()))
        }
        fn byteswap(self) -> Self {
            Self(self.0.swap_bytes())
        }
        fn to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
            unreachable!()
        }
    }

    fn u32_to_char(ch: u32) -> Result<char, String> {
        if ch > 0x10ffff {
            return Err(format!(
                "character U+{:4x} is not in range [U+0000; U+10ffff]",
                ch
            ));
        };
        char::from_u32(ch).ok_or_else(|| {
            format!(
                "'utf-8' codec can't encode character '\\u{:x}' \
                in position 0: surrogates not allowed",
                ch
            )
        })
    }

    impl TryFrom<WideChar> for char {
        type Error = String;

        fn try_from(ch: WideChar) -> Result<Self, Self::Error> {
            // safe because every configuration of bytes for the types we support are valid
            u32_to_char(ch.0 as u32)
        }
    }

    impl IntoPyResult for WideChar {
        fn into_pyresult(self, vm: &VirtualMachine) -> PyResult {
            Ok(
                String::from(char::try_from(self).map_err(|e| vm.new_unicode_encode_error(e))?)
                    .into_pyobject(vm),
            )
        }
    }

    impl fmt::Display for WideChar {
        fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
            unreachable!("`repr(array('u'))` calls `PyStr::repr`")
        }
    }

    #[pyattr]
    #[pyclass(name = "array")]
    #[derive(Debug, PyValue)]
    pub struct PyArray {
        array: PyRwLock<ArrayContentType>,
        exports: AtomicUsize,
    }

    pub type PyArrayRef = PyRef<PyArray>;

    impl From<ArrayContentType> for PyArray {
        fn from(array: ArrayContentType) -> Self {
            PyArray {
                array: PyRwLock::new(array),
                exports: AtomicUsize::new(0),
            }
        }
    }

    #[derive(FromArgs)]
    pub struct ArrayNewArgs {
        #[pyarg(positional)]
        spec: PyStrRef,
        #[pyarg(positional, optional)]
        init: OptionalArg<PyObjectRef>,
    }

    impl Constructor for PyArray {
        type Args = ArrayNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args { spec, init }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let spec = spec.as_str().chars().exactly_one().map_err(|_| {
                vm.new_type_error(
                    "array() argument 1 must be a unicode character, not str".to_owned(),
                )
            })?;
            let mut array =
                ArrayContentType::from_char(spec).map_err(|err| vm.new_value_error(err))?;

            if let OptionalArg::Present(init) = init {
                if let Some(init) = init.payload::<PyArray>() {
                    match (spec, init.read().typecode()) {
                        (spec, ch) if spec == ch => array.frombytes(&init.get_bytes()),
                        (spec, 'u') => {
                            return Err(vm.new_type_error(format!(
                            "cannot use a unicode array to initialize an array with typecode '{}'",
                            spec
                        )))
                        }
                        _ => {
                            for obj in init.read().iter(vm) {
                                array.push(obj?, vm)?;
                            }
                        }
                    }
                } else if let Some(utf8) = init.payload::<PyStr>() {
                    if spec == 'u' {
                        let bytes = Self::_unicode_to_wchar_bytes(utf8.as_str(), array.itemsize());
                        array.frombytes_move(bytes);
                    } else {
                        return Err(vm.new_type_error(format!(
                            "cannot use a str to initialize an array with typecode '{}'",
                            spec
                        )));
                    }
                } else if init.payload_is::<PyBytes>() || init.payload_is::<PyByteArray>() {
                    init.try_bytes_like(vm, |x| array.frombytes(x))?;
                } else if let Ok(iter) = ArgIterable::try_from_object(vm, init.clone()) {
                    for obj in iter.iter(vm)? {
                        array.push(obj?, vm)?;
                    }
                } else {
                    init.try_bytes_like(vm, |x| array.frombytes(x))?;
                }
            }

            let zelf = Self::from(array).into_ref_with_type(vm, cls)?;
            Ok(zelf.into())
        }
    }

    #[pyimpl(
        flags(BASETYPE),
        with(Comparable, AsBuffer, AsMapping, Iterable, Constructor)
    )]
    impl PyArray {
        fn read(&self) -> PyRwLockReadGuard<'_, ArrayContentType> {
            self.array.read()
        }

        fn write(&self) -> PyRwLockWriteGuard<'_, ArrayContentType> {
            self.array.write()
        }

        #[pyproperty]
        fn typecode(&self) -> String {
            self.read().typecode().to_string()
        }

        #[pyproperty]
        fn itemsize(&self) -> usize {
            self.read().itemsize()
        }

        #[pymethod]
        fn append(zelf: PyRef<Self>, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            zelf.try_resizable(vm)?.push(x, vm)
        }

        #[pymethod]
        fn buffer_info(&self) -> (usize, usize) {
            let array = self.read();
            (array.addr(), array.len())
        }

        #[pymethod]
        fn count(&self, x: PyObjectRef, vm: &VirtualMachine) -> usize {
            self.read().count(x, vm)
        }

        #[pymethod]
        fn remove(zelf: PyRef<Self>, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            zelf.try_resizable(vm)?.remove(x, vm)
        }

        #[pymethod]
        fn extend(zelf: PyRef<Self>, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let mut w = zelf.try_resizable(vm)?;
            if zelf.is(&obj) {
                w.imul(2, vm)
            } else if let Some(array) = obj.payload::<PyArray>() {
                w.iadd(&*array.read(), vm)
            } else {
                let iter = ArgIterable::try_from_object(vm, obj)?;
                // zelf.extend_from_iterable(iter, vm)
                for obj in iter.iter(vm)? {
                    w.push(obj?, vm)?;
                }
                Ok(())
            }
        }

        fn _wchar_bytes_to_string(
            bytes: &[u8],
            item_size: usize,
            vm: &VirtualMachine,
        ) -> PyResult<String> {
            if item_size == 2 {
                // safe because every configuration of bytes for the types we support are valid
                let utf16 = unsafe {
                    std::slice::from_raw_parts(
                        bytes.as_ptr() as *const u16,
                        bytes.len() / std::mem::size_of::<u16>(),
                    )
                };
                Ok(String::from_utf16_lossy(utf16))
            } else {
                // safe because every configuration of bytes for the types we support are valid
                let chars = unsafe {
                    std::slice::from_raw_parts(
                        bytes.as_ptr() as *const u32,
                        bytes.len() / std::mem::size_of::<u32>(),
                    )
                };
                chars
                    .iter()
                    .map(|&ch| {
                        // cpython issue 17223
                        u32_to_char(ch).map_err(|msg| vm.new_value_error(msg))
                    })
                    .try_collect()
            }
        }

        fn _unicode_to_wchar_bytes(utf8: &str, item_size: usize) -> Vec<u8> {
            if item_size == 2 {
                utf8.encode_utf16()
                    .flat_map(|ch| ch.to_ne_bytes())
                    .collect()
            } else {
                utf8.chars()
                    .flat_map(|ch| (ch as u32).to_ne_bytes())
                    .collect()
            }
        }

        #[pymethod]
        fn fromunicode(zelf: PyRef<Self>, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let utf8 = PyStrRef::try_from_object(vm, obj.clone()).map_err(|_| {
                vm.new_type_error(format!(
                    "fromunicode() argument must be str, not {}",
                    obj.class().name()
                ))
            })?;
            if zelf.read().typecode() != 'u' {
                return Err(vm.new_value_error(
                    "fromunicode() may only be called on unicode type arrays".into(),
                ));
            }
            let mut w = zelf.try_resizable(vm)?;
            let bytes = Self::_unicode_to_wchar_bytes(utf8.as_str(), w.itemsize());
            w.frombytes_move(bytes);
            Ok(())
        }

        #[pymethod]
        fn tounicode(&self, vm: &VirtualMachine) -> PyResult<String> {
            let array = self.array.read();
            if array.typecode() != 'u' {
                return Err(vm.new_value_error(
                    "tounicode() may only be called on unicode type arrays".into(),
                ));
            }
            let bytes = array.get_bytes();
            Self::_wchar_bytes_to_string(bytes, self.itemsize(), vm)
        }

        fn _from_bytes(&self, b: &[u8], itemsize: usize, vm: &VirtualMachine) -> PyResult<()> {
            if b.len() % itemsize != 0 {
                return Err(
                    vm.new_value_error("bytes length not a multiple of item size".to_owned())
                );
            }
            if b.len() / itemsize > 0 {
                self.try_resizable(vm)?.frombytes(b);
            }
            Ok(())
        }

        #[pymethod]
        fn frombytes(&self, b: ArgBytesLike, vm: &VirtualMachine) -> PyResult<()> {
            let b = b.borrow_buf();
            let itemsize = self.read().itemsize();
            self._from_bytes(&b, itemsize, vm)
        }

        #[pymethod]
        fn fromfile(&self, f: PyObjectRef, n: isize, vm: &VirtualMachine) -> PyResult<()> {
            let itemsize = self.itemsize();
            if n < 0 {
                return Err(vm.new_value_error("negative count".to_owned()));
            }
            let n = vm.check_repeat_or_overflow_error(itemsize, n)?;
            let nbytes = n * itemsize;

            let b = vm.call_method(&f, "read", (nbytes,))?;
            let b = b
                .downcast::<PyBytes>()
                .map_err(|_| vm.new_type_error("read() didn't return bytes".to_owned()))?;

            let not_enough_bytes = b.len() != nbytes;

            self._from_bytes(b.as_bytes(), itemsize, vm)?;

            if not_enough_bytes {
                Err(vm.new_exception_msg(
                    vm.ctx.exceptions.eof_error.clone(),
                    "read() didn't return enough bytes".to_owned(),
                ))
            } else {
                Ok(())
            }
        }

        #[pymethod]
        fn byteswap(&self) {
            self.write().byteswap();
        }

        #[pymethod]
        fn index(&self, x: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
            self.read().index(x, vm)
        }

        #[pymethod]
        fn insert(
            zelf: PyRef<Self>,
            i: isize,
            x: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let mut w = zelf.try_resizable(vm)?;
            w.insert(i, x, vm)
        }

        #[pymethod]
        fn pop(zelf: PyRef<Self>, i: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult {
            let mut w = zelf.try_resizable(vm)?;
            if w.len() == 0 {
                Err(vm.new_index_error("pop from empty array".to_owned()))
            } else {
                w.pop(i.unwrap_or(-1), vm)
            }
        }

        #[pymethod]
        pub(crate) fn tobytes(&self) -> Vec<u8> {
            self.read().get_bytes().to_vec()
        }

        #[pymethod]
        fn tofile(&self, f: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            /* Write 64K blocks at a time */
            /* XXX Make the block size settable */
            const BLOCKSIZE: usize = 64 * 1024;

            let bytes = self.read();
            let bytes = bytes.get_bytes();

            for b in bytes.chunks(BLOCKSIZE) {
                let b = PyBytes::from(b.to_vec()).into_ref(vm);
                vm.call_method(&f, "write", (b,))?;
            }
            Ok(())
        }

        pub(crate) fn get_bytes(&self) -> PyMappedRwLockReadGuard<'_, [u8]> {
            PyRwLockReadGuard::map(self.read(), |a| a.get_bytes())
        }

        pub(crate) fn get_bytes_mut(&self) -> PyMappedRwLockWriteGuard<'_, [u8]> {
            PyRwLockWriteGuard::map(self.write(), |a| a.get_bytes_mut())
        }

        #[pymethod]
        fn tolist(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            let array = self.read();
            let mut v = Vec::with_capacity(array.len());
            for obj in array.iter(vm) {
                v.push(obj?);
            }
            Ok(v)
        }

        #[pymethod]
        fn fromlist(zelf: PyRef<Self>, list: PyListRef, vm: &VirtualMachine) -> PyResult<()> {
            zelf.try_resizable(vm)?.fromlist(&list, vm)
        }

        #[pymethod]
        fn reverse(&self) {
            self.write().reverse()
        }

        #[pymethod(magic)]
        fn copy(&self) -> PyArray {
            self.array.read().clone().into()
        }

        #[pymethod(magic)]
        fn deepcopy(&self, _memo: PyObjectRef) -> PyArray {
            self.copy()
        }

        #[pymethod(magic)]
        fn getitem(&self, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            self.read().getitem(needle, vm)
        }

        #[pymethod(magic)]
        fn setitem(
            zelf: PyRef<Self>,
            needle: PyObjectRef,
            obj: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match SequenceIndex::try_from_object_for(vm, needle, "array")? {
                SequenceIndex::Int(i) => zelf.write().setitem_by_idx(i, obj, vm),
                SequenceIndex::Slice(slice) => {
                    let slice = slice.to_saturated(vm)?;
                    let cloned;
                    let guard;
                    let items = if zelf.is(&obj) {
                        cloned = zelf.read().clone();
                        &cloned
                    } else {
                        match obj.payload::<PyArray>() {
                            Some(array) => {
                                guard = array.read();
                                &*guard
                            }
                            None => {
                                return Err(vm.new_type_error(format!(
                                    "can only assign array (not \"{}\") to array slice",
                                    obj.class().name()
                                )));
                            }
                        }
                    };
                    if let Ok(mut w) = zelf.try_resizable(vm) {
                        w.setitem_by_slice(slice, items, vm)
                    } else {
                        zelf.write().setitem_by_slice_no_resize(slice, items, vm)
                    }
                }
            }
        }

        #[pymethod(magic)]
        fn delitem(zelf: PyRef<Self>, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            match SequenceIndex::try_from_object_for(vm, needle, "array")? {
                SequenceIndex::Int(i) => zelf.try_resizable(vm)?.delitem_by_idx(i, vm),
                SequenceIndex::Slice(slice) => {
                    let slice = slice.to_saturated(vm)?;
                    zelf.try_resizable(vm)?.delitem_by_slice(slice, vm)
                }
            }
        }

        #[pymethod(magic)]
        fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            if let Some(other) = other.payload::<PyArray>() {
                self.read()
                    .add(&*other.read(), vm)
                    .map(|array| PyArray::from(array).into_ref(vm))
            } else {
                Err(vm.new_type_error(format!(
                    "can only append array (not \"{}\") to array",
                    other.class().name()
                )))
            }
        }

        #[pymethod(magic)]
        fn iadd(
            zelf: PyRef<Self>,
            other: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            if zelf.is(&other) {
                zelf.try_resizable(vm)?.imul(2, vm)?;
            } else if let Some(other) = other.payload::<PyArray>() {
                zelf.try_resizable(vm)?.iadd(&*other.read(), vm)?;
            } else {
                return Err(vm.new_type_error(format!(
                    "can only extend array with array (not \"{}\")",
                    other.class().name()
                )));
            }
            Ok(zelf)
        }

        #[pymethod(name = "__rmul__")]
        #[pymethod(magic)]
        fn mul(&self, value: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            self.read()
                .mul(value, vm)
                .map(|x| Self::from(x).into_ref(vm))
        }

        #[pymethod(magic)]
        fn imul(zelf: PyRef<Self>, value: isize, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            zelf.try_resizable(vm)?.imul(value, vm)?;
            Ok(zelf)
        }

        #[pymethod(magic)]
        fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let class = zelf.class();
            let class_name = class.name();
            if zelf.read().typecode() == 'u' {
                if zelf.len() == 0 {
                    return Ok(format!("{}('u')", class_name));
                }
                return Ok(format!(
                    "{}('u', {})",
                    class_name,
                    crate::common::str::repr(&zelf.tounicode(vm)?)
                ));
            }
            zelf.read().repr(&class_name, vm)
        }

        #[pymethod(magic)]
        pub(crate) fn len(&self) -> usize {
            self.read().len()
        }

        fn array_eq(&self, other: &Self, vm: &VirtualMachine) -> PyResult<bool> {
            // we cannot use zelf.is(other) for shortcut because if we contenting a
            // float value NaN we always return False even they are the same object.
            if self.len() != other.len() {
                return Ok(false);
            }
            let array_a = self.read();
            let array_b = other.read();

            // fast path for same ArrayContentType type
            if let Ok(ord) = array_a.cmp(&*array_b) {
                return Ok(ord == Some(Ordering::Equal));
            }

            let iter = Iterator::zip(array_a.iter(vm), array_b.iter(vm));

            for (a, b) in iter {
                if !vm.bool_eq(&*a?, &*b?)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }

        #[pymethod(magic)]
        fn reduce_ex(
            zelf: PyRef<Self>,
            proto: usize,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, PyTupleRef, Option<PyDictRef>)> {
            if proto < 3 {
                return Self::reduce(zelf, vm);
            }
            let array = zelf.read();
            let cls = zelf.as_object().clone_class();
            let typecode = vm.ctx.new_str(array.typecode_str());
            let bytes = vm.ctx.new_bytes(array.get_bytes().to_vec());
            let code = MachineFormatCode::from_typecode(array.typecode()).unwrap();
            let code = PyInt::from(u8::from(code)).into_object(vm);
            let module = vm.import("array", None, 0)?;
            let func = module.get_attr("_array_reconstructor", vm)?;
            Ok((
                func,
                vm.new_tuple((cls, typecode, code, bytes)),
                zelf.as_object().dict(),
            ))
        }

        #[pymethod(magic)]
        fn reduce(
            zelf: PyRef<Self>,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, PyTupleRef, Option<PyDictRef>)> {
            let array = zelf.read();
            let cls = zelf.as_object().clone_class();
            let typecode = vm.ctx.new_str(array.typecode_str());
            let values = if array.typecode() == 'u' {
                let s = Self::_wchar_bytes_to_string(array.get_bytes(), array.itemsize(), vm)?;
                s.chars().map(|x| x.into_pyobject(vm)).collect()
            } else {
                array.get_objects(vm)
            };
            let values = vm.ctx.new_list(values);
            Ok((
                cls.into(),
                vm.new_tuple((typecode, values)),
                zelf.as_object().dict(),
            ))
        }
    }

    impl Comparable for PyArray {
        fn cmp(
            zelf: &PyObjectView<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            // TODO: deduplicate this logic with sequence::cmp in sequence.rs. Maybe make it generic?

            // we cannot use zelf.is(other) for shortcut because if we contenting a
            // float value NaN we always return False even they are the same object.
            let other = class_or_notimplemented!(Self, other);

            if let PyComparisonValue::Implemented(x) =
                op.eq_only(|| Ok(zelf.array_eq(other, vm)?.into()))?
            {
                return Ok(x.into());
            }

            let array_a = zelf.read();
            let array_b = other.read();

            let res = match array_a.cmp(&*array_b) {
                // fast path for same ArrayContentType type
                Ok(partial_ord) => partial_ord.map_or(false, |ord| op.eval_ord(ord)),
                Err(()) => {
                    let iter = Iterator::zip(array_a.iter(vm), array_b.iter(vm));

                    for (a, b) in iter {
                        let ret = match op {
                            PyComparisonOp::Lt | PyComparisonOp::Le => {
                                vm.bool_seq_lt(&*a?, &*b?)?
                            }
                            PyComparisonOp::Gt | PyComparisonOp::Ge => {
                                vm.bool_seq_gt(&*a?, &*b?)?
                            }
                            _ => unreachable!(),
                        };
                        if let Some(v) = ret {
                            return Ok(PyComparisonValue::Implemented(v));
                        }
                    }

                    // fallback:
                    op.eval_ord(array_a.len().cmp(&array_b.len()))
                }
            };

            Ok(res.into())
        }
    }

    impl AsBuffer for PyArray {
        fn as_buffer(zelf: &PyObjectView<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
            let array = zelf.read();
            let buf = PyBuffer::new(
                zelf.to_owned().into(),
                BufferDescriptor::format(
                    array.len() * array.itemsize(),
                    false,
                    array.itemsize(),
                    array.typecode_str().into(),
                ),
                &BUFFER_METHODS,
            );
            Ok(buf)
        }
    }

    static BUFFER_METHODS: BufferMethods = BufferMethods {
        obj_bytes: |buffer| buffer.obj_as::<PyArray>().get_bytes().into(),
        obj_bytes_mut: |buffer| buffer.obj_as::<PyArray>().get_bytes_mut().into(),
        release: |buffer| {
            buffer
                .obj_as::<PyArray>()
                .exports
                .fetch_sub(1, atomic::Ordering::Release);
        },
        retain: |buffer| {
            buffer
                .obj_as::<PyArray>()
                .exports
                .fetch_add(1, atomic::Ordering::Release);
        },
    };

    impl AsMapping for PyArray {
        fn as_mapping(_zelf: &PyObjectView<Self>, _vm: &VirtualMachine) -> PyMappingMethods {
            PyMappingMethods {
                length: Some(Self::length),
                subscript: Some(Self::subscript),
                ass_subscript: Some(Self::ass_subscript),
            }
        }

        fn length(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
            Self::downcast_ref(&zelf, vm).map(|zelf| Ok(zelf.len()))?
        }

        fn subscript(zelf: PyObjectRef, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            Self::downcast_ref(&zelf, vm).map(|zelf| zelf.getitem(needle, vm))?
        }

        fn ass_subscript(
            zelf: PyObjectRef,
            needle: PyObjectRef,
            value: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match value {
                Some(value) => {
                    Self::downcast(zelf, vm).map(|zelf| Self::setitem(zelf, needle, value, vm))?
                }
                None => Self::downcast(zelf, vm).map(|zelf| Self::delitem(zelf, needle, vm))?,
            }
        }
    }

    impl Iterable for PyArray {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyArrayIter {
                position: AtomicUsize::new(0),
                array: zelf,
            }
            .into_object(vm))
        }
    }

    impl<'a> BufferResizeGuard<'a> for PyArray {
        type Resizable = PyRwLockWriteGuard<'a, ArrayContentType>;

        fn try_resizable(&'a self, vm: &VirtualMachine) -> PyResult<Self::Resizable> {
            let w = self.write();
            if self.exports.load(atomic::Ordering::SeqCst) == 0 {
                Ok(w)
            } else {
                Err(vm.new_buffer_error(
                    "Existing exports of data: object cannot be re-sized".to_owned(),
                ))
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "array_iterator")]
    #[derive(Debug, PyValue)]
    pub struct PyArrayIter {
        position: AtomicUsize,
        array: PyArrayRef,
    }

    #[pyimpl(with(IterNext))]
    impl PyArrayIter {}

    impl IterNextIterable for PyArrayIter {}
    impl IterNext for PyArrayIter {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let pos = zelf.position.fetch_add(1, atomic::Ordering::SeqCst);
            let r = if let Some(item) = zelf.array.read().getitem_by_idx(pos, vm)? {
                PyIterReturn::Return(item)
            } else {
                PyIterReturn::StopIteration(None)
            };
            Ok(r)
        }
    }

    #[derive(FromArgs)]
    struct ReconstructorArgs {
        #[pyarg(positional)]
        arraytype: PyTypeRef,
        #[pyarg(positional)]
        typecode: PyStrRef,
        #[pyarg(positional)]
        mformat_code: MachineFormatCode,
        #[pyarg(positional)]
        items: PyBytesRef,
    }

    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    #[repr(u8)]
    enum MachineFormatCode {
        Int8 { signed: bool },                    // 0, 1
        Int16 { signed: bool, big_endian: bool }, // 2, 3, 4, 5
        Int32 { signed: bool, big_endian: bool }, // 6, 7, 8, 9
        Int64 { signed: bool, big_endian: bool }, // 10, 11, 12, 13
        Ieee754Float { big_endian: bool },        // 14, 15
        Ieee754Double { big_endian: bool },       // 16, 17
        Utf16 { big_endian: bool },               // 18, 19
        Utf32 { big_endian: bool },               // 20, 21
    }

    impl From<MachineFormatCode> for u8 {
        fn from(code: MachineFormatCode) -> u8 {
            use MachineFormatCode::*;
            match code {
                Int8 { signed } => signed as u8,
                Int16 { signed, big_endian } => 2 + signed as u8 * 2 + big_endian as u8,
                Int32 { signed, big_endian } => 6 + signed as u8 * 2 + big_endian as u8,
                Int64 { signed, big_endian } => 10 + signed as u8 * 2 + big_endian as u8,
                Ieee754Float { big_endian } => 14 + big_endian as u8,
                Ieee754Double { big_endian } => 16 + big_endian as u8,
                Utf16 { big_endian } => 18 + big_endian as u8,
                Utf32 { big_endian } => 20 + big_endian as u8,
            }
        }
    }

    impl TryFrom<u8> for MachineFormatCode {
        type Error = u8;

        fn try_from(code: u8) -> Result<Self, Self::Error> {
            let big_endian = code % 2 != 0;
            let signed = match code {
                0 | 1 => code != 0,
                2..=13 => (code - 2) % 4 >= 2,
                _ => false,
            };
            match code {
                0 | 1 => Ok(Self::Int8 { signed }),
                2 | 3 | 4 | 5 => Ok(Self::Int16 { signed, big_endian }),
                6 | 7 | 8 | 9 => Ok(Self::Int32 { signed, big_endian }),
                10 | 11 | 12 | 13 => Ok(Self::Int64 { signed, big_endian }),
                14 | 15 => Ok(Self::Ieee754Float { big_endian }),
                16 | 17 => Ok(Self::Ieee754Double { big_endian }),
                18 | 19 => Ok(Self::Utf16 { big_endian }),
                20 | 21 => Ok(Self::Utf32 { big_endian }),
                _ => Err(code),
            }
        }
    }

    impl TryFromObject for MachineFormatCode {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            PyIntRef::try_from_object(vm, obj.clone())
                .map_err(|_| {
                    vm.new_type_error(format!(
                        "an integer is required (got type {})",
                        obj.class().name()
                    ))
                })?
                .try_to_primitive::<i32>(vm)?
                .to_u8()
                .unwrap_or(u8::MAX)
                .try_into()
                .map_err(|_| {
                    vm.new_value_error("third argument must be a valid machine format code.".into())
                })
        }
    }

    impl MachineFormatCode {
        fn from_typecode(code: char) -> Option<Self> {
            use std::mem::size_of;
            let signed = code.is_ascii_uppercase();
            let big_endian = cfg!(target_endian = "big");
            let int_size = match code {
                'b' | 'B' => return Some(Self::Int8 { signed }),
                'u' => {
                    return match size_of::<wchar_t>() {
                        2 => Some(Self::Utf16 { big_endian }),
                        4 => Some(Self::Utf32 { big_endian }),
                        _ => None,
                    }
                }
                'f' => {
                    // Copied from CPython
                    const Y: f32 = 16711938.0;
                    return match &Y.to_ne_bytes() {
                        b"\x4b\x7f\x01\x02" => Some(Self::Ieee754Float { big_endian: true }),
                        b"\x02\x01\x7f\x4b" => Some(Self::Ieee754Float { big_endian: false }),
                        _ => None,
                    };
                }
                'd' => {
                    // Copied from CPython
                    const Y: f64 = 9006104071832581.0;
                    return match &Y.to_ne_bytes() {
                        b"\x43\x3f\xff\x01\x02\x03\x04\x05" => {
                            Some(Self::Ieee754Double { big_endian: true })
                        }
                        b"\x05\x04\x03\x02\x01\xff\x3f\x43" => {
                            Some(Self::Ieee754Double { big_endian: false })
                        }
                        _ => None,
                    };
                }
                _ => ArrayContentType::itemsize_of_typecode(code)? as u8,
            };
            match int_size {
                2 => Some(Self::Int16 { signed, big_endian }),
                4 => Some(Self::Int32 { signed, big_endian }),
                8 => Some(Self::Int64 { signed, big_endian }),
                _ => None,
            }
        }
        fn item_size(self) -> usize {
            match self {
                Self::Int8 { .. } => 1,
                Self::Int16 { .. } | Self::Utf16 { .. } => 2,
                Self::Int32 { .. } | Self::Utf32 { .. } | Self::Ieee754Float { .. } => 4,
                Self::Int64 { .. } | Self::Ieee754Double { .. } => 8,
            }
        }
    }

    fn check_array_type(typ: PyTypeRef, vm: &VirtualMachine) -> PyResult<PyTypeRef> {
        if !typ.issubclass(PyArray::class(vm)) {
            return Err(
                vm.new_type_error(format!("{} is not a subtype of array.array", typ.name()))
            );
        }
        Ok(typ)
    }

    fn check_type_code(spec: PyStrRef, vm: &VirtualMachine) -> PyResult<ArrayContentType> {
        let spec = spec.as_str().chars().exactly_one().map_err(|_| {
            vm.new_type_error(
                "_array_reconstructor() argument 2 must be a unicode character, not str".into(),
            )
        })?;
        ArrayContentType::from_char(spec)
            .map_err(|_| vm.new_value_error("second argument must be a valid type code".into()))
    }

    macro_rules! chunk_to_obj {
        ($BYTE:ident, $TY:ty, $BIG_ENDIAN:ident) => {{
            let b = <[u8; ::std::mem::size_of::<$TY>()]>::try_from($BYTE).unwrap();
            if $BIG_ENDIAN {
                <$TY>::from_be_bytes(b)
            } else {
                <$TY>::from_le_bytes(b)
            }
        }};
        ($VM:ident, $BYTE:ident, $TY:ty, $BIG_ENDIAN:ident) => {
            chunk_to_obj!($BYTE, $TY, $BIG_ENDIAN).into_pyobject($VM)
        };
        ($VM:ident, $BYTE:ident, $SIGNED_TY:ty, $UNSIGNED_TY:ty, $SIGNED:ident, $BIG_ENDIAN:ident) => {{
            let b = <[u8; ::std::mem::size_of::<$SIGNED_TY>()]>::try_from($BYTE).unwrap();
            match ($SIGNED, $BIG_ENDIAN) {
                (false, false) => <$UNSIGNED_TY>::from_le_bytes(b).into_pyobject($VM),
                (false, true) => <$UNSIGNED_TY>::from_be_bytes(b).into_pyobject($VM),
                (true, false) => <$SIGNED_TY>::from_le_bytes(b).into_pyobject($VM),
                (true, true) => <$SIGNED_TY>::from_be_bytes(b).into_pyobject($VM),
            }
        }};
    }

    #[pyfunction]
    fn _array_reconstructor(args: ReconstructorArgs, vm: &VirtualMachine) -> PyResult<PyArrayRef> {
        let cls = check_array_type(args.arraytype, vm)?;
        let mut array = check_type_code(args.typecode, vm)?;
        let format = args.mformat_code;
        let bytes = args.items.as_bytes();
        if bytes.len() % format.item_size() != 0 {
            return Err(vm.new_value_error("bytes length not a multiple of item size".into()));
        }
        if MachineFormatCode::from_typecode(array.typecode()) == Some(format) {
            array.frombytes(bytes);
            return PyArray::from(array).into_ref_with_type(vm, cls);
        }
        if !matches!(
            format,
            MachineFormatCode::Utf16 { .. } | MachineFormatCode::Utf32 { .. }
        ) {
            array.reserve(bytes.len() / format.item_size());
        }
        let mut chunks = bytes.chunks(format.item_size());
        match format {
            MachineFormatCode::Ieee754Float { big_endian } => {
                chunks.try_for_each(|b| array.push(chunk_to_obj!(vm, b, f32, big_endian), vm))?
            }
            MachineFormatCode::Ieee754Double { big_endian } => {
                chunks.try_for_each(|b| array.push(chunk_to_obj!(vm, b, f64, big_endian), vm))?
            }
            MachineFormatCode::Int8 { signed } => chunks
                .try_for_each(|b| array.push(chunk_to_obj!(vm, b, i8, u8, signed, false), vm))?,
            MachineFormatCode::Int16 { signed, big_endian } => chunks.try_for_each(|b| {
                array.push(chunk_to_obj!(vm, b, i16, u16, signed, big_endian), vm)
            })?,
            MachineFormatCode::Int32 { signed, big_endian } => chunks.try_for_each(|b| {
                array.push(chunk_to_obj!(vm, b, i32, u32, signed, big_endian), vm)
            })?,
            MachineFormatCode::Int64 { signed, big_endian } => chunks.try_for_each(|b| {
                array.push(chunk_to_obj!(vm, b, i64, u64, signed, big_endian), vm)
            })?,
            MachineFormatCode::Utf16 { big_endian } => {
                let utf16: Vec<_> = chunks.map(|b| chunk_to_obj!(b, u16, big_endian)).collect();
                let s = String::from_utf16(&utf16).map_err(|_| {
                    vm.new_unicode_encode_error("items cannot decode as utf16".into())
                })?;
                let bytes = PyArray::_unicode_to_wchar_bytes(&s, array.itemsize());
                array.frombytes_move(bytes);
            }
            MachineFormatCode::Utf32 { big_endian } => {
                let s: String = chunks
                    .map(|b| chunk_to_obj!(b, u32, big_endian))
                    .map(|ch| u32_to_char(ch).map_err(|msg| vm.new_value_error(msg)))
                    .try_collect()?;
                let bytes = PyArray::_unicode_to_wchar_bytes(&s, array.itemsize());
                array.frombytes_move(bytes);
            }
        };
        PyArray::from(array).into_ref_with_type(vm, cls)
    }
}
