use super::PyMethod;
use crate::{
    builtins::{pystr::AsPyStr, PyBaseExceptionRef, PyList, PyStrInterned},
    function::IntoFuncArgs,
    identifier,
    object::{AsObject, PyObject, PyObjectRef, PyResult},
    stdlib::sys,
    vm::VirtualMachine,
};

/// PyObject support
impl VirtualMachine {
    #[track_caller]
    #[cold]
    fn _py_panic_failed(&self, exc: PyBaseExceptionRef, msg: &str) -> ! {
        #[cfg(not(all(
            target_arch = "wasm32",
            not(any(target_os = "emscripten", target_os = "wasi")),
        )))]
        {
            self.print_exception(exc);
            self.flush_std();
            panic!("{msg}")
        }
        #[cfg(all(
            target_arch = "wasm32",
            feature = "wasmbind",
            not(any(target_os = "emscripten", target_os = "wasi")),
        ))]
        #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), feature = "wasmbind"))]
        {
            use wasm_bindgen::prelude::*;
            #[wasm_bindgen]
            extern "C" {
                #[wasm_bindgen(js_namespace = console)]
                fn error(s: &str);
            }
            let mut s = String::new();
            self.write_exception(&mut s, &exc).unwrap();
            error(&s);
            panic!("{}; exception backtrace above", msg)
        }
        #[cfg(all(
            target_arch = "wasm32",
            not(feature = "wasmbind"),
            not(any(target_os = "emscripten", target_os = "wasi")),
        ))]
        {
            use crate::convert::ToPyObject;
            let err_string: String = exc.to_pyobject(self).repr(self).unwrap().to_string();
            eprintln!("{err_string}");
            panic!("{}; python exception not available", msg)
        }
    }

    pub(crate) fn flush_std(&self) {
        let vm = self;
        if let Ok(stdout) = sys::get_stdout(vm) {
            let _ = vm.call_method(&stdout, identifier!(vm, flush).as_str(), ());
        }
        if let Ok(stderr) = sys::get_stderr(vm) {
            let _ = vm.call_method(&stderr, identifier!(vm, flush).as_str(), ());
        }
    }

    #[track_caller]
    pub fn unwrap_pyresult<T>(&self, result: PyResult<T>) -> T {
        match result {
            Ok(x) => x,
            Err(exc) => {
                self._py_panic_failed(exc, "called `vm.unwrap_pyresult()` on an `Err` value")
            }
        }
    }
    #[track_caller]
    pub fn expect_pyresult<T>(&self, result: PyResult<T>, msg: &str) -> T {
        match result {
            Ok(x) => x,
            Err(exc) => self._py_panic_failed(exc, msg),
        }
    }

    /// Test whether a python object is `None`.
    pub fn is_none(&self, obj: &PyObject) -> bool {
        obj.is(&self.ctx.none)
    }
    pub fn option_if_none(&self, obj: PyObjectRef) -> Option<PyObjectRef> {
        if self.is_none(&obj) {
            None
        } else {
            Some(obj)
        }
    }
    pub fn unwrap_or_none(&self, obj: Option<PyObjectRef>) -> PyObjectRef {
        obj.unwrap_or_else(|| self.ctx.none())
    }

    pub fn call_get_descriptor_specific(
        &self,
        descr: &PyObject,
        obj: Option<PyObjectRef>,
        cls: Option<PyObjectRef>,
    ) -> Option<PyResult> {
        let descr_get = descr
            .class()
            .mro_find_map(|cls| cls.slots.descr_get.load())?;
        Some(descr_get(descr.to_owned(), obj, cls, self))
    }

    pub fn call_get_descriptor(&self, descr: &PyObject, obj: PyObjectRef) -> Option<PyResult> {
        let cls = obj.class().to_owned().into();
        self.call_get_descriptor_specific(descr, Some(obj), Some(cls))
    }

    pub fn call_if_get_descriptor(&self, attr: &PyObject, obj: PyObjectRef) -> PyResult {
        self.call_get_descriptor(attr, obj)
            .unwrap_or_else(|| Ok(attr.to_owned()))
    }

    #[inline]
    pub fn call_method<T>(&self, obj: &PyObject, method_name: &str, args: T) -> PyResult
    where
        T: IntoFuncArgs,
    {
        flame_guard!(format!("call_method({:?})", method_name));

        let dynamic_name;
        let name = match self.ctx.interned_str(method_name) {
            Some(name) => name.as_pystr(&self.ctx),
            None => {
                dynamic_name = self.ctx.new_str(method_name);
                &dynamic_name
            }
        };
        PyMethod::get(obj.to_owned(), name, self)?.invoke(args, self)
    }

    pub fn dir(&self, obj: Option<PyObjectRef>) -> PyResult<PyList> {
        let seq = match obj {
            Some(obj) => self
                .get_special_method(&obj, identifier!(self, __dir__))?
                .ok_or_else(|| self.new_type_error("object does not provide __dir__".to_owned()))?
                .invoke((), self)?,
            None => self.call_method(
                self.current_locals()?.as_object(),
                identifier!(self, keys).as_str(),
                (),
            )?,
        };
        let items: Vec<_> = seq.try_to_value(self)?;
        let lst = PyList::from(items);
        lst.sort(Default::default(), self)?;
        Ok(lst)
    }

    #[inline]
    pub(crate) fn get_special_method(
        &self,
        obj: &PyObject,
        method: &'static PyStrInterned,
    ) -> PyResult<Option<PyMethod>> {
        PyMethod::get_special::<false>(obj, method, self)
    }

    /// NOT PUBLIC API
    #[doc(hidden)]
    pub fn call_special_method(
        &self,
        obj: &PyObject,
        method: &'static PyStrInterned,
        args: impl IntoFuncArgs,
    ) -> PyResult {
        self.get_special_method(obj, method)?
            .ok_or_else(|| self.new_attribute_error(method.as_str().to_owned()))?
            .invoke(args, self)
    }

    /// Same as __builtins__.print in Python.
    /// A convenience function to provide a simple way to print objects for debug purpose.
    // NOTE: Keep the interface simple.
    pub fn print(&self, args: impl IntoFuncArgs) -> PyResult<()> {
        let ret = self.builtins.get_attr("print", self)?.call(args, self)?;
        debug_assert!(self.is_none(&ret));
        Ok(())
    }

    #[deprecated(note = "in favor of `obj.call(args, vm)`")]
    pub fn invoke(&self, obj: &impl AsObject, args: impl IntoFuncArgs) -> PyResult {
        obj.as_object().call(args, self)
    }
}
