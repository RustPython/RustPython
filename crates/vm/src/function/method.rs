use crate::{
    Context, Py, PyObjectRef, PyPayload, PyRef, VirtualMachine,
    builtins::{
        PyType,
        builtin_func::{PyNativeFunction, PyNativeMethod},
        descriptor::PyMethodDescriptor,
    },
    function::{IntoPyNativeFn, PyNativeFn},
};

bitflags::bitflags! {
    // METH_XXX flags in CPython
    #[derive(Copy, Clone, Debug, PartialEq)]
    pub struct PyMethodFlags: u32 {
        // const VARARGS = 0x0001;
        // const KEYWORDS = 0x0002;
        // METH_NOARGS and METH_O must not be combined with the flags above.
        // const NOARGS = 0x0004;
        // const O = 0x0008;

        // METH_CLASS and METH_STATIC are a little different; these control
        // the construction of methods for a class.  These cannot be used for
        // functions in modules.
        const CLASS = 0x0010;
        const STATIC = 0x0020;

        // METH_COEXIST allows a method to be entered even though a slot has
        // already filled the entry.  When defined, the flag allows a separate
        // method, "__contains__" for example, to coexist with a defined
        // slot like sq_contains.
        // const COEXIST = 0x0040;

        // if not Py_LIMITED_API
        // const FASTCALL = 0x0080;

        // This bit is preserved for Stackless Python
        // const STACKLESS = 0x0100;

        // METH_METHOD means the function stores an
        // additional reference to the class that defines it;
        // both self and class are passed to it.
        // It uses PyCMethodObject instead of PyCFunctionObject.
        // May not be combined with METH_NOARGS, METH_O, METH_CLASS or METH_STATIC.
        const METHOD = 0x0200;
    }
}

impl PyMethodFlags {
    // FIXME: macro temp
    pub const EMPTY: Self = Self::empty();
}

#[macro_export]
macro_rules! define_methods {
    // TODO: more flexible syntax
    ($($name:literal => $func:ident as $flags:ident),+) => {
        vec![ $( $crate::function::PyMethodDef {
            name: $name,
            func: $crate::function::static_func($func),
            flags: $crate::function::PyMethodFlags::$flags,
            doc: None,
        }),+ ]
    };
}

#[derive(Clone)]
pub struct PyMethodDef {
    pub name: &'static str, // TODO: interned
    pub func: &'static dyn PyNativeFn,
    pub flags: PyMethodFlags,
    pub doc: Option<&'static str>, // TODO: interned
}

impl PyMethodDef {
    #[inline]
    pub const fn new_const<Kind>(
        name: &'static str,
        func: impl IntoPyNativeFn<Kind>,
        flags: PyMethodFlags,
        doc: Option<&'static str>,
    ) -> Self {
        Self {
            name,
            func: super::static_func(func),
            flags,
            doc,
        }
    }

    #[inline]
    pub const fn new_raw_const(
        name: &'static str,
        func: impl PyNativeFn,
        flags: PyMethodFlags,
        doc: Option<&'static str>,
    ) -> Self {
        Self {
            name,
            func: super::static_raw_func(func),
            flags,
            doc,
        }
    }

    pub fn to_proper_method(
        &'static self,
        class: &'static Py<PyType>,
        ctx: &Context,
    ) -> PyObjectRef {
        if self.flags.contains(PyMethodFlags::METHOD) {
            self.build_method(ctx, class).into()
        } else if self.flags.contains(PyMethodFlags::CLASS) {
            self.build_classmethod(ctx, class).into()
        } else if self.flags.contains(PyMethodFlags::STATIC) {
            self.build_staticmethod(ctx, class).into()
        } else {
            unreachable!()
        }
    }

    pub const fn to_function(&'static self) -> PyNativeFunction {
        PyNativeFunction {
            zelf: None,
            value: self,
            module: None,
        }
    }

    pub fn to_method(
        &'static self,
        class: &'static Py<PyType>,
        ctx: &Context,
    ) -> PyMethodDescriptor {
        PyMethodDescriptor::new(self, class, ctx)
    }

    pub const fn to_bound_method(
        &'static self,
        obj: PyObjectRef,
        class: &'static Py<PyType>,
    ) -> PyNativeMethod {
        PyNativeMethod {
            func: PyNativeFunction {
                zelf: Some(obj),
                value: self,
                module: None,
            },
            class,
        }
    }

    pub fn build_function(&'static self, ctx: &Context) -> PyRef<PyNativeFunction> {
        self.to_function().into_ref(ctx)
    }

    pub fn build_bound_function(
        &'static self,
        ctx: &Context,
        obj: PyObjectRef,
    ) -> PyRef<PyNativeFunction> {
        let function = PyNativeFunction {
            zelf: Some(obj),
            value: self,
            module: None,
        };
        PyRef::new_ref(
            function,
            ctx.types.builtin_function_or_method_type.to_owned(),
            None,
        )
    }

    pub fn build_method(
        &'static self,
        ctx: &Context,
        class: &'static Py<PyType>,
    ) -> PyRef<PyMethodDescriptor> {
        debug_assert!(self.flags.contains(PyMethodFlags::METHOD));
        let method = self.to_method(class, ctx);
        PyRef::new_ref(method, ctx.types.method_descriptor_type.to_owned(), None)
    }

    pub fn build_bound_method(
        &'static self,
        ctx: &Context,
        obj: PyObjectRef,
        class: &'static Py<PyType>,
    ) -> PyRef<PyNativeMethod> {
        PyRef::new_ref(
            self.to_bound_method(obj, class),
            ctx.types.builtin_method_type.to_owned(),
            None,
        )
    }

    pub fn build_classmethod(
        &'static self,
        ctx: &Context,
        class: &'static Py<PyType>,
    ) -> PyRef<PyMethodDescriptor> {
        PyRef::new_ref(
            self.to_method(class, ctx),
            ctx.types.method_descriptor_type.to_owned(),
            None,
        )
    }

    pub fn build_staticmethod(
        &'static self,
        ctx: &Context,
        class: &'static Py<PyType>,
    ) -> PyRef<PyNativeMethod> {
        debug_assert!(self.flags.contains(PyMethodFlags::STATIC));
        let func = self.to_function();
        PyNativeMethod { func, class }.into_ref(ctx)
    }

    #[doc(hidden)]
    pub const fn __const_concat_arrays<const SUM_LEN: usize>(
        method_groups: &[&[Self]],
    ) -> [Self; SUM_LEN] {
        const NULL_METHOD: PyMethodDef = PyMethodDef {
            name: "",
            func: &|_, _| unreachable!(),
            flags: PyMethodFlags::empty(),
            doc: None,
        };
        let mut all_methods = [NULL_METHOD; SUM_LEN];
        let mut all_idx = 0;
        let mut group_idx = 0;
        while group_idx < method_groups.len() {
            let group = method_groups[group_idx];
            let mut method_idx = 0;
            while method_idx < group.len() {
                all_methods[all_idx] = group[method_idx].const_copy();
                method_idx += 1;
                all_idx += 1;
            }
            group_idx += 1;
        }
        all_methods
    }

    const fn const_copy(&self) -> Self {
        Self {
            name: self.name,
            func: self.func,
            flags: self.flags,
            doc: self.doc,
        }
    }
}

impl core::fmt::Debug for PyMethodDef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PyMethodDef")
            .field("name", &self.name)
            .field(
                "func",
                &(unsafe {
                    core::mem::transmute::<&dyn PyNativeFn, [usize; 2]>(self.func)[1] as *const u8
                }),
            )
            .field("flags", &self.flags)
            .field("doc", &self.doc)
            .finish()
    }
}

// This is not a part of CPython API.
// But useful to support dynamically generated methods
#[pyclass(name, module = false, ctx = "method_def")]
#[derive(Debug)]
pub struct HeapMethodDef {
    method: PyMethodDef,
}

impl HeapMethodDef {
    pub const fn new(method: PyMethodDef) -> Self {
        Self { method }
    }
}

impl Py<HeapMethodDef> {
    pub(crate) unsafe fn method(&self) -> &'static PyMethodDef {
        unsafe { &*(&self.method as *const _) }
    }

    pub fn build_function(&self, vm: &VirtualMachine) -> PyRef<PyNativeFunction> {
        let function = unsafe { self.method() }.to_function();
        let dict = vm.ctx.new_dict();
        dict.set_item("__method_def__", self.to_owned().into(), vm)
            .unwrap();
        PyRef::new_ref(
            function,
            vm.ctx.types.builtin_function_or_method_type.to_owned(),
            Some(dict),
        )
    }

    pub fn build_method(
        &self,
        class: &'static Py<PyType>,
        vm: &VirtualMachine,
    ) -> PyRef<PyMethodDescriptor> {
        let function = unsafe { self.method() }.to_method(class, &vm.ctx);
        let dict = vm.ctx.new_dict();
        dict.set_item("__method_def__", self.to_owned().into(), vm)
            .unwrap();
        PyRef::new_ref(
            function,
            vm.ctx.types.method_descriptor_type.to_owned(),
            Some(dict),
        )
    }
}

#[pyclass]
impl HeapMethodDef {}
