use super::PyMethod;
use crate::{
    builtins::{PyBaseExceptionRef, PyList, PyListRef, PyStrInterned, pystr::AsPyStr},
    function::IntoFuncArgs,
    object::{AsObject, PyObject, PyObjectRef, PyResult},
    vm::VirtualMachine,
};

/// PyObject support
impl VirtualMachine {
    /// PyObject_Dump
    ///
    /// Write a description of `obj` to the process stderr, bypassing
    /// `sys.stderr`.
    pub fn dump_object(&self, obj: &PyObject) {
        let class = obj.class();
        eprintln!("object address  : {obj:p}");
        eprintln!("object refcount : {}", obj.strong_count());
        eprintln!("object type     : {class:p}");
        eprintln!("object type name: {}", class.slot_name());
        eprint!("object repr     : ");
        if let Ok(repr) = obj.repr(self) {
            eprint!("{}", repr.as_wtf8());
        }
        eprintln!();
    }

    #[track_caller]
    #[cold]
    fn _py_panic_failed(&self, exc: PyBaseExceptionRef, msg: &str) -> ! {
        cfg_select! {
            all(
                target_arch = "wasm32",
                not(any(target_os = "emscripten", target_os = "wasi"))
            ) => cfg_select! {
                feature = "wasmbind" => {
                    use wasm_bindgen::prelude::*;
                    #[wasm_bindgen]
                    extern "C" {
                        #[wasm_bindgen(js_namespace = console)]
                        fn error(s: &str);
                    }
                    let mut s = String::new();
                    self.write_exception(&mut s, &exc).unwrap();
                    error(&s);
                    panic!("{msg}; exception backtrace above")
                }
                _ => {
                    use crate::convert::ToPyObject;
                    let err_string: String = exc.to_pyobject(self).repr(self).unwrap().to_string();
                    eprintln!("{err_string}");
                    panic!("{msg}; python exception not available")
                }
            },
            _ => {
                self.print_exception(&exc);
                self.flush_std();
                panic!("{msg}")
            }
        }
    }

    /// Returns true if the file object's `closed` attribute is truthy.
    fn file_is_closed(&self, file: &PyObject) -> bool {
        file.get_attr("closed", self)
            .is_ok_and(|v| v.try_to_bool(self).unwrap_or_default())
    }

    pub(crate) fn flush_std(&self) -> i32 {
        let mut status = 0;
        for name in ["stdout", "stderr"] {
            // Keep the stream alive until its failure has been reported: a
            // flush or hook can replace the reference stored in sys.__dict__.
            let file = self.sys_module.dict().get_item_opt(name, self);
            let result = match &file {
                Ok(Some(file)) if !self.is_none(file) && !self.file_is_closed(file) => {
                    self.call_method(file, "flush", ()).map(drop)
                }
                Ok(_) => Ok(()),
                Err(error) => Err(error.clone()),
            };
            if let Err(error) = result {
                status = -1;
                if name == "stdout" {
                    self.run_unraisable(
                        error,
                        Some("Exception ignored while flushing sys.stdout".to_owned()),
                        self.ctx.none(),
                    );
                }
                // Reporting a stderr failure could require the broken stream.
            }
        }
        status
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
        if self.is_none(&obj) { None } else { Some(obj) }
    }

    pub fn unwrap_or_none(&self, obj: Option<PyObjectRef>) -> PyObjectRef {
        obj.unwrap_or_else(|| self.ctx.none())
    }

    pub fn call_get_descriptor_specific(
        &self,
        descr: &PyObject,
        obj: Option<&PyObject>,
        cls: Option<&PyObject>,
    ) -> Option<PyResult> {
        let descr_get = descr.class().slots().descr_get.load()?;
        Some(descr_get(descr, obj, cls, self))
    }

    pub fn call_get_descriptor(&self, descr: &PyObject, obj: &PyObject) -> Option<PyResult> {
        self.call_get_descriptor_specific(descr, Some(obj), Some(obj.class().as_object()))
    }

    pub fn call_if_get_descriptor(&self, attr: &PyObject, obj: PyObjectRef) -> PyResult {
        self.call_get_descriptor(attr, &obj)
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

    pub fn dir(&self, obj: Option<PyObjectRef>) -> PyResult<PyListRef> {
        // Keep the input alive until collection and sorting have both finished.
        let lst = match obj.as_ref() {
            Some(obj) => {
                let seq =
                    PyMethod::get_special_ex::<false>(obj, identifier!(self, __dir__), self, true)?
                        .ok_or_else(|| self.new_type_error("object does not provide __dir__"))?
                        .invoke((), self)?;
                // PySequence_List asks the returned iterable for its length hint.
                let items = self.extract_elements_sized(&seq, &|| 0, Ok);
                drop(seq);
                self.ctx.new_list(items?)
            }
            None => {
                let locals = self.current_locals()?;
                let keys = locals.mapping().keys(self)?;
                // Mapping keys preserve an exact list returned by user code.
                // The exact-dict fast path instead supplies a keys view.
                match keys.downcast::<PyList>() {
                    Ok(list) => list,
                    Err(keys) => {
                        self.ctx
                            .new_list(self.extract_elements_sized(&keys, &|| 0, Ok)?)
                    }
                }
            }
        };
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

    /// Call a special method, treating AttributeError from descriptor binding
    /// as a missing method. Errors raised by the method itself are preserved.
    #[doc(hidden)]
    pub fn call_special_method_opt(
        &self,
        obj: &PyObject,
        method: &'static PyStrInterned,
        args: impl IntoFuncArgs,
    ) -> PyResult<Option<PyObjectRef>> {
        self.get_special_method(obj, method)?
            .map(|method| method.invoke(args, self))
            .transpose()
    }

    /// NOT PUBLIC API
    #[doc(hidden)]
    pub fn call_special_method(
        &self,
        obj: &PyObject,
        method: &'static PyStrInterned,
        args: impl IntoFuncArgs,
    ) -> PyResult {
        // lookup_method: AttributeError from a data descriptor is kept.
        PyMethod::get_special_ex::<false>(obj, method, self, true)?
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
