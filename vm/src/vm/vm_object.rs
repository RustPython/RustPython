use super::PyMethod;
use crate::{
    builtins::{PyBaseExceptionRef, PyList, PyStr, PyStrInterned},
    function::IntoFuncArgs,
    identifier,
    object::{AsObject, PyObject, PyObjectRef, PyPayload, PyResult},
    vm::VirtualMachine,
};

/// PyObject support
impl VirtualMachine {
    #[track_caller]
    #[cold]
    fn _py_panic_failed(&self, exc: PyBaseExceptionRef, msg: &str) -> ! {
        #[cfg(not(all(target_arch = "wasm32", not(target_os = "wasi"))))]
        {
            let show_backtrace =
                std::env::var_os("RUST_BACKTRACE").map_or(cfg!(target_os = "wasi"), |v| &v != "0");
            let after = if show_backtrace {
                self.print_exception(exc);
                "exception backtrace above"
            } else {
                "run with RUST_BACKTRACE=1 to see Python backtrace"
            };
            panic!("{msg}; {after}")
        }
        #[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
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
        descr: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: Option<PyObjectRef>,
    ) -> Result<PyResult, PyObjectRef> {
        let descr_get = descr.class().mro_find_map(|cls| cls.slots.descr_get.load());
        match descr_get {
            Some(descr_get) => Ok(descr_get(descr, obj, cls, self)),
            None => Err(descr),
        }
    }

    pub fn call_get_descriptor(
        &self,
        descr: PyObjectRef,
        obj: PyObjectRef,
    ) -> Result<PyResult, PyObjectRef> {
        let cls = obj.class().to_owned().into();
        self.call_get_descriptor_specific(descr, Some(obj), Some(cls))
    }

    pub fn call_if_get_descriptor(&self, attr: PyObjectRef, obj: PyObjectRef) -> PyResult {
        self.call_get_descriptor(attr, obj).unwrap_or_else(Ok)
    }

    #[inline]
    pub fn call_method<T>(&self, obj: &PyObject, method_name: &str, args: T) -> PyResult
    where
        T: IntoFuncArgs,
    {
        flame_guard!(format!("call_method({:?})", method_name));

        let name = self
            .ctx
            .interned_str(method_name)
            .map_or_else(|| PyStr::from(method_name).into_ref(self), |s| s.to_owned());
        PyMethod::get(obj.to_owned(), name, self)?.invoke(args, self)
    }

    pub fn dir(&self, obj: Option<PyObjectRef>) -> PyResult<PyList> {
        let seq = match obj {
            Some(obj) => self
                .get_special_method(obj, identifier!(self, __dir__))?
                .map_err(|_obj| self.new_type_error("object does not provide __dir__".to_owned()))?
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
        obj: PyObjectRef,
        method: &'static PyStrInterned,
    ) -> PyResult<Result<PyMethod, PyObjectRef>> {
        PyMethod::get_special(obj, method, self)
    }

    /// NOT PUBLIC API
    #[doc(hidden)]
    pub fn call_special_method(
        &self,
        obj: PyObjectRef,
        method: &'static PyStrInterned,
        args: impl IntoFuncArgs,
    ) -> PyResult {
        self.get_special_method(obj, method)?
            .map_err(|_obj| self.new_attribute_error(method.as_str().to_owned()))?
            .invoke(args, self)
    }

    // #[deprecated(note = "in favor of `obj.call(args, vm)`")]
    pub fn invoke(&self, obj: &impl AsObject, args: impl IntoFuncArgs) -> PyResult {
        obj.as_object().call(args, self)
    }
}
