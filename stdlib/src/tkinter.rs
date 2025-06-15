// cspell:ignore createcommand

pub(crate) use self::_tkinter::make_module;

#[pymodule]
mod _tkinter {
    use rustpython_vm::types::Constructor;
    use rustpython_vm::{PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine};

    use rustpython_vm::builtins::{PyInt, PyStr, PyType};
    use std::{ffi, ptr};

    use crate::builtins::PyTypeRef;
    use rustpython_common::atomic::AtomicBool;
    use rustpython_common::atomic::Ordering;

    #[cfg(windows)]
    fn _get_tcl_lib_path() -> String {
        // TODO: fix packaging
        String::from(r"C:\ActiveTcl\lib")
    }

    #[pyattr(name = "TclError", once)]
    fn tcl_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "_tkinter",
            "TclError",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
        )
    }

    #[pyattr(name = "TkError", once)]
    fn tk_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "_tkinter",
            "TkError",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
        )
    }

    #[pyattr(once, name = "TK_VERSION")]
    fn tk_version(_vm: &VirtualMachine) -> String {
        format!("{}.{}", 8, 6)
    }

    #[pyattr(once, name = "TCL_VERSION")]
    fn tcl_version(_vm: &VirtualMachine) -> String {
        format!(
            "{}.{}",
            tk_sys::TCL_MAJOR_VERSION,
            tk_sys::TCL_MINOR_VERSION
        )
    }

    #[pyattr]
    #[pyclass(name = "TclObject")]
    #[derive(PyPayload)]
    struct TclObject {
        value: *mut tk_sys::Tcl_Obj,
    }

    impl std::fmt::Debug for TclObject {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "TclObject")
        }
    }

    unsafe impl Send for TclObject {}
    unsafe impl Sync for TclObject {}

    #[pyclass]
    impl TclObject {}

    static QUIT_MAIN_LOOP: AtomicBool = AtomicBool::new(false);
    static ERROR_IN_CMD: AtomicBool = AtomicBool::new(false);

    #[pyattr]
    #[pyclass(name = "tkapp")]
    #[derive(PyPayload)]
    struct TkApp {
        // Tcl_Interp *interp;
        interpreter: *mut tk_sys::Tcl_Interp,
        // int wantobjects;
        want_objects: bool,
        // int threaded; /* True if tcl_platform[threaded] */
        threaded: bool,
        // Tcl_ThreadId thread_id;
        thread_id: Option<tk_sys::Tcl_ThreadId>,
        // int dispatching;
        dispatching: bool,
        // PyObject *trace;
        trace: Option<()>,
        // /* We cannot include tclInt.h, as this is internal.
        //    So we cache interesting types here. */
        old_boolean_type: *const tk_sys::Tcl_ObjType,
        boolean_type: *const tk_sys::Tcl_ObjType,
        byte_array_type: *const tk_sys::Tcl_ObjType,
        double_type: *const tk_sys::Tcl_ObjType,
        int_type: *const tk_sys::Tcl_ObjType,
        wide_int_type: *const tk_sys::Tcl_ObjType,
        bignum_type: *const tk_sys::Tcl_ObjType,
        list_type: *const tk_sys::Tcl_ObjType,
        string_type: *const tk_sys::Tcl_ObjType,
        utf32_string_type: *const tk_sys::Tcl_ObjType,
        pixel_type: *const tk_sys::Tcl_ObjType,
    }

    unsafe impl Send for TkApp {}
    unsafe impl Sync for TkApp {}

    impl std::fmt::Debug for TkApp {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "TkApp")
        }
    }

    #[derive(FromArgs, Debug)]
    struct TkAppConstructorArgs {
        #[pyarg(any)]
        screen_name: Option<String>,
        #[pyarg(any)]
        _base_name: Option<String>,
        #[pyarg(any)]
        class_name: String,
        #[pyarg(any)]
        interactive: i32,
        #[pyarg(any)]
        wantobjects: i32,
        #[pyarg(any, default = true)]
        want_tk: bool,
        #[pyarg(any)]
        sync: i32,
        #[pyarg(any)]
        use_: Option<String>,
    }

    impl Constructor for TkApp {
        type Args = TkAppConstructorArgs;

        fn py_new(
            _zelf: PyRef<PyType>,
            args: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            create(args, vm)
        }
    }

    fn varname_converter(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
        // if let Ok(bytes) = obj.bytes(vm) {
        //     todo!()
        // }

        // str
        if let Some(str) = obj.downcast_ref::<PyStr>() {
            return Ok(str.as_str().to_string());
        }

        if let Some(_tcl_obj) = obj.downcast_ref::<TclObject>() {
            // Assume that the Tcl object has a method to retrieve a string.
            // return tcl_obj.
            todo!();
        }

        // Construct an error message using the type name (truncated to 50 characters).
        Err(vm.new_type_error(format!(
            "must be str, bytes or Tcl_Obj, not {:.50}",
            obj.obj_type().str(vm)?.as_str()
        )))
    }

    #[derive(Debug, FromArgs)]
    struct TkAppGetVarArgs {
        #[pyarg(any)]
        name: PyObjectRef,
        #[pyarg(any, default)]
        name2: Option<String>,
    }

    // TODO: DISALLOW_INSTANTIATION
    #[pyclass(with(Constructor))]
    impl TkApp {
        fn from_bool(&self, obj: *mut tk_sys::Tcl_Obj) -> bool {
            let mut res = -1;
            unsafe {
                if tk_sys::Tcl_GetBooleanFromObj(self.interpreter, obj, &mut res)
                    != tk_sys::TCL_OK as i32
                {
                    panic!("Tcl_GetBooleanFromObj failed");
                }
            }
            assert!(res == 0 || res == 1);
            res != 0
        }

        fn from_object(
            &self,
            obj: *mut tk_sys::Tcl_Obj,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            let type_ptr = unsafe { (*obj).typePtr };
            if type_ptr == ptr::null() {
                return self.unicode_from_object(obj, vm);
            } else if type_ptr == self.old_boolean_type || type_ptr == self.boolean_type {
                return Ok(vm.ctx.new_bool(self.from_bool(obj)).into());
            } else if type_ptr == self.string_type
                || type_ptr == self.utf32_string_type
                || type_ptr == self.pixel_type
            {
                return self.unicode_from_object(obj, vm);
            }
            // TODO: handle other types

            return Ok(TclObject { value: obj }.into_pyobject(vm));
        }

        fn unicode_from_string(
            s: *mut ffi::c_char,
            size: usize,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            // terribly unsafe
            let s = unsafe { std::slice::from_raw_parts(s, size) }
                .to_vec()
                .into_iter()
                .map(|c| c as u8)
                .collect::<Vec<u8>>();
            let s = String::from_utf8(s).unwrap();
            Ok(PyObjectRef::from(vm.ctx.new_str(s)))
        }

        fn unicode_from_object(
            &self,
            obj: *mut tk_sys::Tcl_Obj,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            let type_ptr = unsafe { (*obj).typePtr };
            if type_ptr != ptr::null()
                && self.interpreter != ptr::null_mut()
                && (type_ptr == self.string_type || type_ptr == self.utf32_string_type)
            {
                let len = ptr::null_mut();
                let data = unsafe { tk_sys::Tcl_GetUnicodeFromObj(obj, len) };
                return if size_of::<tk_sys::Tcl_UniChar>() == 2 {
                    let v = unsafe { std::slice::from_raw_parts(data as *const u16, len as usize) };
                    let s = String::from_utf16(v).unwrap();
                    Ok(PyObjectRef::from(vm.ctx.new_str(s)))
                } else {
                    let v = unsafe { std::slice::from_raw_parts(data as *const u32, len as usize) };
                    let s = widestring::U32String::from_vec(v).to_string_lossy();
                    Ok(PyObjectRef::from(vm.ctx.new_str(s)))
                };
            }
            let len = ptr::null_mut();
            let s = unsafe { tk_sys::Tcl_GetStringFromObj(obj, len) };
            Self::unicode_from_string(s, len as _, vm)
        }

        fn var_invoke(&self) {
            if self.threaded && self.thread_id != unsafe { tk_sys::Tcl_GetCurrentThread() } {
                // TODO: do stuff
            }
        }

        fn inner_getvar(
            &self,
            args: TkAppGetVarArgs,
            flags: u32,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            let TkAppGetVarArgs { name, name2 } = args;
            // TODO: technically not thread safe
            let name = varname_converter(name, vm)?;

            let name = ffi::CString::new(name)?;
            let name2 = ffi::CString::new(name2.unwrap_or_default())?;
            let name2_ptr = if name2.is_empty() {
                ptr::null()
            } else {
                name2.as_ptr()
            };
            let res = unsafe {
                tk_sys::Tcl_GetVar2Ex(
                    self.interpreter,
                    name.as_ptr() as _,
                    name2_ptr as _,
                    flags as _,
                )
            };
            if res == ptr::null_mut() {
                // TODO: Should be tk error
                unsafe {
                    let err_obj = tk_sys::Tcl_GetObjResult(self.interpreter);
                    let err_str_obj = tk_sys::Tcl_GetString(err_obj);
                    let err_cstr = ffi::CStr::from_ptr(err_str_obj as _);
                    return Err(vm.new_type_error(format!("{err_cstr:?}")));
                }
            }
            let res = if self.want_objects {
                self.from_object(res, vm)
            } else {
                self.unicode_from_object(res, vm)
            }?;
            Ok(res)
        }

        #[pymethod]
        fn getvar(&self, args: TkAppGetVarArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            self.var_invoke();
            self.inner_getvar(args, tk_sys::TCL_LEAVE_ERR_MSG, vm)
        }

        #[pymethod]
        fn globalgetvar(
            &self,
            args: TkAppGetVarArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            self.var_invoke();
            self.inner_getvar(
                args,
                tk_sys::TCL_LEAVE_ERR_MSG | tk_sys::TCL_GLOBAL_ONLY,
                vm,
            )
        }

        #[pymethod]
        fn getint(&self, arg: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            if let Some(int) = arg.downcast_ref::<PyInt>() {
                return Ok(PyObjectRef::from(vm.ctx.new_int(int.as_bigint().clone())));
            }

            if let Some(obj) = arg.downcast_ref::<TclObject>() {
                let value = obj.value;
                unsafe { tk_sys::Tcl_IncrRefCount(value) };
            } else {
                todo!();
            }
            todo!();
        }
        // TODO: Fix arguments
        #[pymethod]
        fn mainloop(&self, threshold: Option<i32>) -> PyResult<()> {
            let threshold = threshold.unwrap_or(0);
            // self.dispatching = true;
            QUIT_MAIN_LOOP.store(false, Ordering::Relaxed);
            while unsafe { tk_sys::Tk_GetNumMainWindows() } > threshold
                && !QUIT_MAIN_LOOP.load(Ordering::Relaxed)
                && !ERROR_IN_CMD.load(Ordering::Relaxed)
            {
                let mut result = 0;
                if self.threaded {
                    result = unsafe { tk_sys::Tcl_DoOneEvent(0 as _) } as i32;
                } else {
                    result = unsafe { tk_sys::Tcl_DoOneEvent(tk_sys::TCL_DONT_WAIT as _) } as i32;
                    // TODO: sleep for the proper time
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
            Ok(())
        }

        #[pymethod]
        fn quit(&self) {
            QUIT_MAIN_LOOP.store(true, Ordering::Relaxed);
        }
    }

    #[pyfunction]
    fn create(args: TkAppConstructorArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        unsafe {
            let interp = tk_sys::Tcl_CreateInterp();
            let want_objects = args.wantobjects != 0;
            let threaded = {
                let part1 = String::from("tcl_platform");
                let part2 = String::from("threaded");
                let part1 = ffi::CString::new(part1)?;
                let part2 = ffi::CString::new(part2)?;
                let part1_ptr = part1.as_ptr();
                let part2_ptr = part2.as_ptr();
                tk_sys::Tcl_GetVar2Ex(
                    interp,
                    part1_ptr as _,
                    part2_ptr as _,
                    tk_sys::TCL_GLOBAL_ONLY as ffi::c_int,
                )
            } != ptr::null_mut();
            let thread_id = tk_sys::Tcl_GetCurrentThread();
            let dispatching = false;
            let trace = None;
            // TODO: Handle threaded build
            let bool_str = String::from("oldBoolean");
            let old_boolean_type = tk_sys::Tcl_GetObjType(bool_str.as_ptr() as _);
            let (boolean_type, byte_array_type) = {
                let true_str = String::from("true");
                let mut value = *tk_sys::Tcl_NewStringObj(true_str.as_ptr() as _, -1);
                let mut bool_value = 0;
                tk_sys::Tcl_GetBooleanFromObj(interp, &mut value, &mut bool_value);
                let boolean_type = value.typePtr;
                tk_sys::Tcl_DecrRefCount(&mut value);

                let mut value =
                    *tk_sys::Tcl_NewByteArrayObj(&bool_value as *const i32 as *const u8, 1);
                let byte_array_type = value.typePtr;
                tk_sys::Tcl_DecrRefCount(&mut value);
                (boolean_type, byte_array_type)
            };
            let double_str = String::from("double");
            let double_type = tk_sys::Tcl_GetObjType(double_str.as_ptr() as _);
            let int_str = String::from("int");
            let int_type = tk_sys::Tcl_GetObjType(int_str.as_ptr() as _);
            let int_type = if int_type == ptr::null() {
                let mut value = *tk_sys::Tcl_NewIntObj(0);
                let res = value.typePtr;
                tk_sys::Tcl_DecrRefCount(&mut value);
                res
            } else {
                int_type
            };
            let wide_int_str = String::from("wideInt");
            let wide_int_type = tk_sys::Tcl_GetObjType(wide_int_str.as_ptr() as _);
            let bignum_str = String::from("bignum");
            let bignum_type = tk_sys::Tcl_GetObjType(bignum_str.as_ptr() as _);
            let list_str = String::from("list");
            let list_type = tk_sys::Tcl_GetObjType(list_str.as_ptr() as _);
            let string_str = String::from("string");
            let string_type = tk_sys::Tcl_GetObjType(string_str.as_ptr() as _);
            let utf32_str = String::from("utf32");
            let utf32_string_type = tk_sys::Tcl_GetObjType(utf32_str.as_ptr() as _);
            let pixel_str = String::from("pixel");
            let pixel_type = tk_sys::Tcl_GetObjType(pixel_str.as_ptr() as _);

            let exit_str = String::from("exit");
            tk_sys::Tcl_DeleteCommand(interp, exit_str.as_ptr() as _);

            if let Some(name) = args.screen_name {
                tk_sys::Tcl_SetVar2(
                    interp,
                    "env".as_ptr() as _,
                    "DISPLAY".as_ptr() as _,
                    name.as_ptr() as _,
                    tk_sys::TCL_GLOBAL_ONLY as i32,
                );
            }

            if args.interactive != 0 {
                tk_sys::Tcl_SetVar(
                    interp,
                    "tcl_interactive".as_ptr() as _,
                    "1".as_ptr() as _,
                    tk_sys::TCL_GLOBAL_ONLY as i32,
                );
            } else {
                tk_sys::Tcl_SetVar(
                    interp,
                    "tcl_interactive".as_ptr() as _,
                    "0".as_ptr() as _,
                    tk_sys::TCL_GLOBAL_ONLY as i32,
                );
            }

            let argv0 = args.class_name.clone().to_lowercase();
            tk_sys::Tcl_SetVar(
                interp,
                "argv0".as_ptr() as _,
                argv0.as_ptr() as _,
                tk_sys::TCL_GLOBAL_ONLY as i32,
            );

            if !args.want_tk {
                tk_sys::Tcl_SetVar(
                    interp,
                    "_tkinter_skip_tk_init".as_ptr() as _,
                    "1".as_ptr() as _,
                    tk_sys::TCL_GLOBAL_ONLY as i32,
                );
            }

            if args.sync != 0 || args.use_.is_some() {
                let mut argv = String::with_capacity(4);
                if args.sync != 0 {
                    argv.push_str("-sync");
                }
                if args.use_.is_some() {
                    if args.sync != 0 {
                        argv.push(' ');
                    }
                    argv.push_str("-use ");
                    argv.push_str(&args.use_.unwrap());
                }
                argv.push_str("\0");
                let argv_ptr = argv.as_ptr() as *mut *mut i8;
                tk_sys::Tcl_SetVar(
                    interp,
                    "argv".as_ptr() as _,
                    argv_ptr as *const i8,
                    tk_sys::TCL_GLOBAL_ONLY as i32,
                );
            }

            #[cfg(windows)]
            {
                let ret = std::env::var("TCL_LIBRARY");
                if ret.is_err() {
                    let loc = _get_tcl_lib_path();
                    std::env::set_var("TCL_LIBRARY", loc);
                }
            }

            // Bindgen cannot handle Tcl_AppInit
            if tk_sys::Tcl_Init(interp) != tk_sys::TCL_OK as ffi::c_int {
                todo!("Tcl_Init failed");
            }

            Ok(TkApp {
                interpreter: interp,
                want_objects,
                threaded,
                thread_id: Some(thread_id),
                dispatching,
                trace,
                old_boolean_type,
                boolean_type,
                byte_array_type,
                double_type,
                int_type,
                wide_int_type,
                bignum_type,
                list_type,
                string_type,
                utf32_string_type,
                pixel_type,
            }
            .into_pyobject(vm))
        }
    }

    #[pyattr]
    const READABLE: i32 = tk_sys::TCL_READABLE as i32;
    #[pyattr]
    const WRITABLE: i32 = tk_sys::TCL_WRITABLE as i32;
    #[pyattr]
    const EXCEPTION: i32 = tk_sys::TCL_EXCEPTION as i32;

    #[pyattr]
    const TIMER_EVENTS: i32 = tk_sys::TCL_TIMER_EVENTS as i32;
    #[pyattr]
    const IDLE_EVENTS: i32 = tk_sys::TCL_IDLE_EVENTS as i32;
    #[pyattr]
    const DONT_WAIT: i32 = tk_sys::TCL_DONT_WAIT as i32;
}
