use rustpython_vm::{PyObjectRef, VirtualMachine};

// pub(crate) use _sqlite::make_module;
pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    // TODO: sqlite version check
    let module = _sqlite::make_module(vm);
    _sqlite::setup_module(&module, vm);
    module
}

#[pymodule]
mod _sqlite {
    use libsqlite3_sys::{
        sqlite3, sqlite3_aggregate_context, sqlite3_backup_finish, sqlite3_backup_init,
        sqlite3_backup_pagecount, sqlite3_backup_remaining, sqlite3_backup_step, sqlite3_bind_blob,
        sqlite3_bind_double, sqlite3_bind_int64, sqlite3_bind_null, sqlite3_bind_parameter_count,
        sqlite3_bind_parameter_name, sqlite3_bind_text, sqlite3_blob, sqlite3_blob_bytes,
        sqlite3_blob_close, sqlite3_blob_open, sqlite3_blob_read, sqlite3_blob_write,
        sqlite3_busy_timeout, sqlite3_changes, sqlite3_close_v2, sqlite3_column_blob,
        sqlite3_column_bytes, sqlite3_column_count, sqlite3_column_decltype, sqlite3_column_double,
        sqlite3_column_int64, sqlite3_column_name, sqlite3_column_text, sqlite3_column_type,
        sqlite3_complete, sqlite3_context, sqlite3_context_db_handle, sqlite3_create_collation_v2,
        sqlite3_create_function_v2, sqlite3_create_window_function, sqlite3_data_count,
        sqlite3_db_handle, sqlite3_errcode, sqlite3_errmsg, sqlite3_exec, sqlite3_expanded_sql,
        sqlite3_extended_errcode, sqlite3_finalize, sqlite3_get_autocommit, sqlite3_interrupt,
        sqlite3_last_insert_rowid, sqlite3_libversion, sqlite3_limit, sqlite3_open_v2,
        sqlite3_prepare_v2, sqlite3_progress_handler, sqlite3_reset, sqlite3_result_blob,
        sqlite3_result_double, sqlite3_result_error, sqlite3_result_error_nomem,
        sqlite3_result_error_toobig, sqlite3_result_int64, sqlite3_result_null,
        sqlite3_result_text, sqlite3_set_authorizer, sqlite3_sleep, sqlite3_step, sqlite3_stmt,
        sqlite3_stmt_busy, sqlite3_stmt_readonly, sqlite3_threadsafe, sqlite3_total_changes,
        sqlite3_trace_v2, sqlite3_user_data, sqlite3_value, sqlite3_value_blob,
        sqlite3_value_bytes, sqlite3_value_double, sqlite3_value_int64, sqlite3_value_text,
        sqlite3_value_type, SQLITE_BLOB, SQLITE_DETERMINISTIC, SQLITE_FLOAT, SQLITE_INTEGER,
        SQLITE_NULL, SQLITE_OPEN_CREATE, SQLITE_OPEN_READWRITE, SQLITE_OPEN_URI, SQLITE_TEXT,
        SQLITE_TRACE_STMT, SQLITE_TRANSIENT, SQLITE_UTF8,
    };
    use rustpython_common::{
        atomic::{Ordering, PyAtomic, Radium},
        hash::PyHash,
        lock::{PyMappedMutexGuard, PyMutex, PyMutexGuard},
        static_cell,
    };
    use rustpython_vm::{
        atomic_func,
        builtins::{
            PyBaseException, PyBaseExceptionRef, PyByteArray, PyBytes, PyDict, PyDictRef, PyFloat,
            PyInt, PyIntRef, PySlice, PyStr, PyStrRef, PyTuple, PyTupleRef, PyType, PyTypeRef,
        },
        convert::IntoObject,
        function::{ArgCallable, ArgIterable, FuncArgs, OptionalArg, PyComparisonValue},
        protocol::{PyBuffer, PyIterReturn, PyMappingMethods, PySequence, PySequenceMethods},
        sliceable::{SaturatedSliceIter, SliceableSequenceOp},
        stdlib::os::PyPathLike,
        types::{
            AsMapping, AsSequence, Callable, Comparable, Constructor, Hashable, IterNext,
            IterNextIterable, Iterable, PyComparisonOp,
        },
        utils::ToCString,
        AsObject, Py, PyAtomicRef, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
        TryFromBorrowedObject, VirtualMachine,
        __exports::paste,
    };
    use std::{
        ffi::{c_int, c_longlong, c_uint, c_void, CStr},
        fmt::Debug,
        ops::Deref,
        ptr::{addr_of_mut, null, null_mut},
        thread::ThreadId,
    };

    macro_rules! exceptions {
        ($(($x:ident, $base:expr)),*) => {
            paste::paste! {
                static_cell! {
                    $(
                        static [<$x:snake:upper>]: PyTypeRef;
                    )*
                }
                $(
                    #[allow(dead_code)]
                    fn [<new_ $x:snake>](vm: &VirtualMachine, msg: String) -> PyBaseExceptionRef {
                        vm.new_exception_msg([<$x:snake _type>]().to_owned(), msg)
                    }
                    fn [<$x:snake _type>]() -> &'static Py<PyType> {
                        [<$x:snake:upper>].get().expect("exception type not initialize")
                    }
                )*
                fn setup_module_exceptions(module: &PyObject, vm: &VirtualMachine) {
                    $(
                        let exception = [<$x:snake:upper>].get_or_init(
                            || vm.ctx.new_exception_type("_sqlite3", stringify!($x), Some(vec![$base(vm).to_owned()])));
                        module.set_attr(stringify!($x), exception.clone().into_object(), vm).unwrap();
                    )*
                }
            }
        };
    }

    exceptions!(
        (Warning, |vm: &VirtualMachine| vm
            .ctx
            .exceptions
            .exception_type),
        (Error, |vm: &VirtualMachine| vm
            .ctx
            .exceptions
            .exception_type),
        (InterfaceError, |_| error_type()),
        (DatabaseError, |_| error_type()),
        (DataError, |_| database_error_type()),
        (OperationalError, |_| database_error_type()),
        (IntegrityError, |_| database_error_type()),
        (InternalError, |_| database_error_type()),
        (ProgrammingError, |_| database_error_type()),
        (NotSupportedError, |_| database_error_type())
    );

    #[pyattr]
    fn sqlite_version(vm: &VirtualMachine) -> String {
        let s = unsafe { sqlite3_libversion() };
        ptr_to_str(s, vm).unwrap().to_owned()
    }

    #[pyattr]
    fn threadsafety(_: &VirtualMachine) -> c_int {
        let mode = unsafe { sqlite3_threadsafe() };
        match mode {
            0 => 0,
            1 => 3,
            2 => 1,
            _ => panic!("Unable to interpret SQLite threadsafety mode"),
        }
    }

    #[pyattr(name = "_deprecated_version")]
    const PYSQLITE_VERSION: &str = "2.6.0";

    #[pyattr]
    const PARSE_DECLTYPES: c_int = 1;
    #[pyattr]
    const PARSE_COLNAMES: c_int = 2;

    #[pyattr]
    use libsqlite3_sys::{
        SQLITE_ALTER_TABLE, SQLITE_ANALYZE, SQLITE_ATTACH, SQLITE_CREATE_INDEX,
        SQLITE_CREATE_TABLE, SQLITE_CREATE_TEMP_INDEX, SQLITE_CREATE_TEMP_TABLE,
        SQLITE_CREATE_TEMP_TRIGGER, SQLITE_CREATE_TEMP_VIEW, SQLITE_CREATE_TRIGGER,
        SQLITE_CREATE_VIEW, SQLITE_CREATE_VTABLE, SQLITE_DELETE, SQLITE_DENY, SQLITE_DETACH,
        SQLITE_DROP_INDEX, SQLITE_DROP_TABLE, SQLITE_DROP_TEMP_INDEX, SQLITE_DROP_TEMP_TABLE,
        SQLITE_DROP_TEMP_TRIGGER, SQLITE_DROP_TEMP_VIEW, SQLITE_DROP_TRIGGER, SQLITE_DROP_VIEW,
        SQLITE_DROP_VTABLE, SQLITE_FUNCTION, SQLITE_IGNORE, SQLITE_INSERT, SQLITE_LIMIT_ATTACHED,
        SQLITE_LIMIT_COLUMN, SQLITE_LIMIT_COMPOUND_SELECT, SQLITE_LIMIT_EXPR_DEPTH,
        SQLITE_LIMIT_FUNCTION_ARG, SQLITE_LIMIT_LENGTH, SQLITE_LIMIT_LIKE_PATTERN_LENGTH,
        SQLITE_LIMIT_SQL_LENGTH, SQLITE_LIMIT_TRIGGER_DEPTH, SQLITE_LIMIT_VARIABLE_NUMBER,
        SQLITE_LIMIT_VDBE_OP, SQLITE_LIMIT_WORKER_THREADS, SQLITE_PRAGMA, SQLITE_READ,
        SQLITE_RECURSIVE, SQLITE_REINDEX, SQLITE_SAVEPOINT, SQLITE_SELECT, SQLITE_TRANSACTION,
        SQLITE_UPDATE,
    };

    macro_rules! error_codes {
        ($($x:ident),*) => {
            $(
                #[allow(unused_imports)]
                use libsqlite3_sys::$x;
            )*
            static ERROR_CODES: &[(&str, c_int)] = &[
            $(
                (stringify!($x), libsqlite3_sys::$x),
            )*
            ];
        };
    }

    error_codes!(
        SQLITE_ABORT,
        SQLITE_AUTH,
        SQLITE_BUSY,
        SQLITE_CANTOPEN,
        SQLITE_CONSTRAINT,
        SQLITE_CORRUPT,
        SQLITE_DONE,
        SQLITE_EMPTY,
        SQLITE_ERROR,
        SQLITE_FORMAT,
        SQLITE_FULL,
        SQLITE_INTERNAL,
        SQLITE_INTERRUPT,
        SQLITE_IOERR,
        SQLITE_LOCKED,
        SQLITE_MISMATCH,
        SQLITE_MISUSE,
        SQLITE_NOLFS,
        SQLITE_NOMEM,
        SQLITE_NOTADB,
        SQLITE_NOTFOUND,
        SQLITE_OK,
        SQLITE_PERM,
        SQLITE_PROTOCOL,
        SQLITE_RANGE,
        SQLITE_READONLY,
        SQLITE_ROW,
        SQLITE_SCHEMA,
        SQLITE_TOOBIG,
        SQLITE_NOTICE,
        SQLITE_WARNING,
        SQLITE_ABORT_ROLLBACK,
        SQLITE_BUSY_RECOVERY,
        SQLITE_CANTOPEN_FULLPATH,
        SQLITE_CANTOPEN_ISDIR,
        SQLITE_CANTOPEN_NOTEMPDIR,
        SQLITE_CORRUPT_VTAB,
        SQLITE_IOERR_ACCESS,
        SQLITE_IOERR_BLOCKED,
        SQLITE_IOERR_CHECKRESERVEDLOCK,
        SQLITE_IOERR_CLOSE,
        SQLITE_IOERR_DELETE,
        SQLITE_IOERR_DELETE_NOENT,
        SQLITE_IOERR_DIR_CLOSE,
        SQLITE_IOERR_DIR_FSYNC,
        SQLITE_IOERR_FSTAT,
        SQLITE_IOERR_FSYNC,
        SQLITE_IOERR_LOCK,
        SQLITE_IOERR_NOMEM,
        SQLITE_IOERR_RDLOCK,
        SQLITE_IOERR_READ,
        SQLITE_IOERR_SEEK,
        SQLITE_IOERR_SHMLOCK,
        SQLITE_IOERR_SHMMAP,
        SQLITE_IOERR_SHMOPEN,
        SQLITE_IOERR_SHMSIZE,
        SQLITE_IOERR_SHORT_READ,
        SQLITE_IOERR_TRUNCATE,
        SQLITE_IOERR_UNLOCK,
        SQLITE_IOERR_WRITE,
        SQLITE_LOCKED_SHAREDCACHE,
        SQLITE_READONLY_CANTLOCK,
        SQLITE_READONLY_RECOVERY,
        SQLITE_CONSTRAINT_CHECK,
        SQLITE_CONSTRAINT_COMMITHOOK,
        SQLITE_CONSTRAINT_FOREIGNKEY,
        SQLITE_CONSTRAINT_FUNCTION,
        SQLITE_CONSTRAINT_NOTNULL,
        SQLITE_CONSTRAINT_PRIMARYKEY,
        SQLITE_CONSTRAINT_TRIGGER,
        SQLITE_CONSTRAINT_UNIQUE,
        SQLITE_CONSTRAINT_VTAB,
        SQLITE_READONLY_ROLLBACK,
        SQLITE_IOERR_MMAP,
        SQLITE_NOTICE_RECOVER_ROLLBACK,
        SQLITE_NOTICE_RECOVER_WAL,
        SQLITE_BUSY_SNAPSHOT,
        SQLITE_IOERR_GETTEMPPATH,
        SQLITE_WARNING_AUTOINDEX,
        SQLITE_CANTOPEN_CONVPATH,
        SQLITE_IOERR_CONVPATH,
        SQLITE_CONSTRAINT_ROWID,
        SQLITE_READONLY_DBMOVED,
        SQLITE_AUTH_USER,
        SQLITE_OK_LOAD_PERMANENTLY,
        SQLITE_IOERR_VNODE,
        SQLITE_IOERR_AUTH,
        SQLITE_IOERR_BEGIN_ATOMIC,
        SQLITE_IOERR_COMMIT_ATOMIC,
        SQLITE_IOERR_ROLLBACK_ATOMIC,
        SQLITE_ERROR_MISSING_COLLSEQ,
        SQLITE_ERROR_RETRY,
        SQLITE_READONLY_CANTINIT,
        SQLITE_READONLY_DIRECTORY,
        SQLITE_CORRUPT_SEQUENCE,
        SQLITE_LOCKED_VTAB,
        SQLITE_CANTOPEN_DIRTYWAL,
        SQLITE_ERROR_SNAPSHOT,
        SQLITE_CANTOPEN_SYMLINK,
        SQLITE_CONSTRAINT_PINNED,
        SQLITE_OK_SYMLINK,
        SQLITE_BUSY_TIMEOUT,
        SQLITE_CORRUPT_INDEX,
        SQLITE_IOERR_DATA,
        SQLITE_IOERR_CORRUPTFS
    );

    #[derive(FromArgs)]
    struct ConnectArgs {
        #[pyarg(any)]
        database: PyPathLike,
        #[pyarg(any, default = "5.0")]
        timeout: f64,
        #[pyarg(any, default = "0")]
        detect_types: c_int,
        #[pyarg(any, default = "Some(vm.ctx.empty_str.clone())")]
        isolation_level: Option<PyStrRef>,
        #[pyarg(any, default = "true")]
        check_same_thread: bool,
        #[pyarg(any, default = "Connection::class(vm).to_owned()")]
        factory: PyTypeRef,
        // TODO: cache statements
        #[allow(dead_code)]
        #[pyarg(any, default = "0")]
        cached_statements: c_int,
        #[pyarg(any, default = "false")]
        uri: bool,
    }

    #[derive(FromArgs)]
    struct BackupArgs {
        #[pyarg(any)]
        target: PyRef<Connection>,
        #[pyarg(named, default = "-1")]
        pages: c_int,
        #[pyarg(named, optional)]
        progress: Option<ArgCallable>,
        #[pyarg(named, optional)]
        name: Option<PyStrRef>,
        #[pyarg(named, default = "0.250")]
        sleep: f64,
    }

    #[derive(FromArgs)]
    struct CreateFunctionArgs {
        #[pyarg(any)]
        name: PyStrRef,
        #[pyarg(any)]
        narg: c_int,
        #[pyarg(any)]
        func: PyObjectRef,
        #[pyarg(named, default)]
        deterministic: bool,
    }

    #[derive(FromArgs)]
    struct CreateAggregateArgs {
        #[pyarg(any)]
        name: PyStrRef,
        #[pyarg(positional)]
        narg: c_int,
        #[pyarg(positional)]
        aggregate_class: PyObjectRef,
    }

    #[derive(FromArgs)]
    struct BlobOpenArgs {
        #[pyarg(positional)]
        table: PyStrRef,
        #[pyarg(positional)]
        column: PyStrRef,
        #[pyarg(positional)]
        row: i64,
        #[pyarg(named, default)]
        readonly: bool,
        #[pyarg(named, default = "vm.ctx.new_str(stringify!(main))")]
        name: PyStrRef,
    }

    struct CallbackData {
        obj: *const PyObject,
        vm: *const VirtualMachine,
    }

    impl CallbackData {
        fn new(obj: PyObjectRef, vm: &VirtualMachine) -> Option<Self> {
            (!vm.is_none(&obj)).then_some(Self {
                obj: obj.into_raw(),
                vm,
            })
        }

        fn retrive(&self) -> (&PyObject, &VirtualMachine) {
            unsafe { (&*self.obj, &*self.vm) }
        }

        unsafe extern "C" fn destructor(data: *mut c_void) {
            drop(Box::from_raw(data.cast::<Self>()));
        }

        unsafe extern "C" fn func_callback(
            context: *mut sqlite3_context,
            argc: c_int,
            argv: *mut *mut sqlite3_value,
        ) {
            let context = SqliteContext::from(context);
            let (func, vm) = (*context.user_data::<Self>()).retrive();
            let args = std::slice::from_raw_parts(argv, argc as usize);

            let f = || -> PyResult<()> {
                let db = context.db_handle();
                let args = args
                    .iter()
                    .cloned()
                    .map(|val| value_to_object(val, db, vm))
                    .collect::<PyResult<Vec<PyObjectRef>>>()?;

                let val = vm.invoke(func, args)?;

                context.result_from_object(&val, vm)
            };

            if let Err(exc) = f() {
                context.result_exception(vm, exc, "user-defined function raised exception\0")
            }
        }

        unsafe extern "C" fn step_callback(
            context: *mut sqlite3_context,
            argc: c_int,
            argv: *mut *mut sqlite3_value,
        ) {
            let context = SqliteContext::from(context);
            let (cls, vm) = (*context.user_data::<Self>()).retrive();
            let args = std::slice::from_raw_parts(argv, argc as usize);
            let instance = context.aggregate_context::<*const PyObject>();
            if (*instance).is_null() {
                match vm.invoke(cls, ()) {
                    Ok(obj) => *instance = obj.into_raw(),
                    Err(exc) => {
                        return context.result_exception(
                            vm,
                            exc,
                            "user-defined aggregate's '__init__' method raised error\0",
                        )
                    }
                }
            }
            let instance = &**instance;

            Self::call_method_with_args(context, instance, "step", args, vm);
        }

        unsafe extern "C" fn finalize_callback(context: *mut sqlite3_context) {
            let context = SqliteContext::from(context);
            let (_, vm) = (*context.user_data::<Self>()).retrive();
            let instance = context.aggregate_context::<*const PyObject>();
            let Some(instance) = (*instance).as_ref() else { return; };

            Self::callback_result_from_method(context, instance, "finalize", vm);
        }

        unsafe extern "C" fn collation_callback(
            data: *mut c_void,
            a_len: c_int,
            a_ptr: *const c_void,
            b_len: c_int,
            b_ptr: *const c_void,
        ) -> c_int {
            let (callable, vm) = (*data.cast::<Self>()).retrive();

            let f = || -> PyResult<c_int> {
                let text1 = ptr_to_string(a_ptr.cast(), a_len, null_mut(), vm)?;
                let text1 = vm.ctx.new_str(text1);
                let text2 = ptr_to_string(b_ptr.cast(), b_len, null_mut(), vm)?;
                let text2 = vm.ctx.new_str(text2);

                let val = vm.invoke(callable, (text1, text2))?;
                let Some(val) = val.to_number().index(vm) else {
                    return Ok(0);
                };

                let val = match val?.as_bigint().sign() {
                    num_bigint::Sign::Plus => 1,
                    num_bigint::Sign::Minus => -1,
                    num_bigint::Sign::NoSign => 0,
                };

                Ok(val)
            };

            f().unwrap_or(0)
        }

        unsafe extern "C" fn value_callback(context: *mut sqlite3_context) {
            let context = SqliteContext::from(context);
            let (_, vm) = (*context.user_data::<Self>()).retrive();
            let instance = context.aggregate_context::<*const PyObject>();
            let instance = &**instance;

            Self::callback_result_from_method(context, instance, "value", vm);
        }

        unsafe extern "C" fn inverse_callback(
            context: *mut sqlite3_context,
            argc: c_int,
            argv: *mut *mut sqlite3_value,
        ) {
            let context = SqliteContext::from(context);
            let (_, vm) = (*context.user_data::<Self>()).retrive();
            let args = std::slice::from_raw_parts(argv, argc as usize);
            let instance = context.aggregate_context::<*const PyObject>();
            let instance = &**instance;

            Self::call_method_with_args(context, instance, "inverse", args, vm);
        }

        unsafe extern "C" fn authorizer_callback(
            data: *mut c_void,
            action: c_int,
            arg1: *const libc::c_char,
            arg2: *const libc::c_char,
            db_name: *const libc::c_char,
            access: *const libc::c_char,
        ) -> c_int {
            let (callable, vm) = (*data.cast::<Self>()).retrive();
            let f = || -> PyResult<c_int> {
                let arg1 = ptr_to_str(arg1, vm)?;
                let arg2 = ptr_to_str(arg2, vm)?;
                let db_name = ptr_to_str(db_name, vm)?;
                let access = ptr_to_str(access, vm)?;

                let val = vm.invoke(callable, (action, arg1, arg2, db_name, access))?;
                let Some(val) = val.payload::<PyInt>() else {
                    return Ok(SQLITE_DENY);
                };
                val.try_to_primitive::<c_int>(vm)
            };

            f().unwrap_or(SQLITE_DENY)
        }

        unsafe extern "C" fn trace_callback(
            _typ: c_uint,
            data: *mut c_void,
            stmt: *mut c_void,
            sql: *mut c_void,
        ) -> c_int {
            let (callable, vm) = (*data.cast::<Self>()).retrive();
            let expanded = sqlite3_expanded_sql(stmt.cast());
            let f = || -> PyResult<()> {
                let stmt = ptr_to_str(expanded, vm).or_else(|_| ptr_to_str(sql.cast(), vm))?;
                vm.invoke(callable, (stmt,)).map(drop)
            };
            let _ = f();
            0
        }

        unsafe extern "C" fn progress_callback(data: *mut c_void) -> c_int {
            let (callable, vm) = (*data.cast::<Self>()).retrive();
            if let Ok(val) = vm.invoke(callable, ()) {
                if let Ok(val) = val.is_true(vm) {
                    return val as c_int;
                }
            }
            -1
        }

        fn callback_result_from_method(
            context: SqliteContext,
            instance: &PyObject,
            name: &str,
            vm: &VirtualMachine,
        ) {
            let f = || -> PyResult<()> {
                let val = vm.call_method(instance, name, ())?;
                context.result_from_object(&val, vm)
            };

            if let Err(exc) = f() {
                if exc.fast_isinstance(vm.ctx.exceptions.attribute_error) {
                    context.result_exception(
                        vm,
                        exc,
                        &format!("user-defined aggregate's '{name}' method not defined\0"),
                    )
                } else {
                    context.result_exception(
                        vm,
                        exc,
                        &format!("user-defined aggregate's '{name}' method raised error\0"),
                    )
                }
            }
        }

        fn call_method_with_args(
            context: SqliteContext,
            instance: &PyObject,
            name: &str,
            args: &[*mut sqlite3_value],
            vm: &VirtualMachine,
        ) {
            let f = || -> PyResult<()> {
                let db = context.db_handle();
                let args = args
                    .iter()
                    .cloned()
                    .map(|val| value_to_object(val, db, vm))
                    .collect::<PyResult<Vec<PyObjectRef>>>()?;
                vm.call_method(instance, name, args).map(drop)
            };

            if let Err(exc) = f() {
                if exc.fast_isinstance(vm.ctx.exceptions.attribute_error) {
                    context.result_exception(
                        vm,
                        exc,
                        &format!("user-defined aggregate's '{name}' method not defined\0"),
                    )
                } else {
                    context.result_exception(
                        vm,
                        exc,
                        &format!("user-defined aggregate's '{name}' method raised error\0"),
                    )
                }
            }
        }
    }

    impl Drop for CallbackData {
        fn drop(&mut self) {
            unsafe { PyObjectRef::from_raw(self.obj) };
        }
    }

    #[pyfunction]
    fn connect(args: ConnectArgs, vm: &VirtualMachine) -> PyResult {
        Connection::py_new(args.factory.clone(), args, vm)
    }

    #[pyfunction]
    fn complete_statement(statement: PyStrRef, vm: &VirtualMachine) -> PyResult<bool> {
        let s = statement.to_cstring(vm)?;
        let ret = unsafe { sqlite3_complete(s.as_ptr()) };
        Ok(ret == 1)
    }

    #[pyfunction]
    fn enable_callback_tracebacks(flag: bool) {
        enable_traceback().store(flag, Ordering::Relaxed);
    }

    #[pyfunction]
    fn register_adapter(typ: PyTypeRef, adapter: ArgCallable, vm: &VirtualMachine) -> PyResult<()> {
        if typ.is(PyInt::class(vm))
            || typ.is(PyFloat::class(vm))
            || typ.is(PyStr::class(vm))
            || typ.is(PyByteArray::class(vm))
        {
            let _ = BASE_TYPE_ADAPTED.set(());
        }
        let protocol = PrepareProtocol::class(vm).to_owned();
        let key = vm.ctx.new_tuple(vec![typ.into(), protocol.into()]);
        adapters().set_item(key.as_object(), adapter.into(), vm)
    }

    #[pyfunction]
    fn register_converter(
        typename: PyStrRef,
        converter: ArgCallable,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let name = typename.as_str().to_uppercase();
        converters().set_item(&name, converter.into(), vm)
    }

    fn _adapt<F>(obj: &PyObject, proto: PyTypeRef, alt: F, vm: &VirtualMachine) -> PyResult
    where
        F: FnOnce(&PyObject) -> PyResult,
    {
        let proto = proto.into_object();
        let key = vm
            .ctx
            .new_tuple(vec![obj.class().to_owned().into(), proto.clone()]);

        if let Some(adapter) = adapters().get_item_opt(key.as_object(), vm)? {
            return vm.invoke(&adapter, (obj,));
        }
        if let Ok(adapter) = proto.get_attr("__adapt__", vm) {
            match vm.invoke(&adapter, (obj,)) {
                Ok(val) => return Ok(val),
                Err(exc) => {
                    if !exc.fast_isinstance(vm.ctx.exceptions.type_error) {
                        return Err(exc);
                    }
                }
            }
        }
        if let Ok(adapter) = obj.get_attr("__conform__", vm) {
            match vm.invoke(&adapter, (proto,)) {
                Ok(val) => return Ok(val),
                Err(exc) => {
                    if !exc.fast_isinstance(vm.ctx.exceptions.type_error) {
                        return Err(exc);
                    }
                }
            }
        }

        alt(obj)
    }

    #[pyfunction]
    fn adapt(
        obj: PyObjectRef,
        proto: OptionalArg<Option<PyTypeRef>>,
        alt: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        // TODO: None proto
        let proto = proto
            .flatten()
            .unwrap_or_else(|| PrepareProtocol::class(vm).to_owned());

        _adapt(
            &obj,
            proto,
            |_| {
                if let OptionalArg::Present(alt) = alt {
                    Ok(alt)
                } else {
                    Err(new_programming_error(vm, "can't adapt".to_owned()))
                }
            },
            vm,
        )
    }

    fn need_adapt(obj: &PyObject, vm: &VirtualMachine) -> bool {
        if BASE_TYPE_ADAPTED.get().is_some() {
            true
        } else {
            let cls = obj.class();
            !(cls.is(vm.ctx.types.int_type)
                || cls.is(vm.ctx.types.float_type)
                || cls.is(vm.ctx.types.str_type)
                || cls.is(vm.ctx.types.bytearray_type))
        }
    }

    static_cell! {
        static CONVERTERS: PyDictRef;
        static ADAPTERS: PyDictRef;
        static BASE_TYPE_ADAPTED: ();
        static USER_FUNCTION_EXCEPTION: PyAtomicRef<Option<PyBaseException>>;
        static ENABLE_TRACEBACK: PyAtomic<bool>;
    }

    fn converters() -> &'static Py<PyDict> {
        CONVERTERS.get().expect("converters not initialize")
    }

    fn adapters() -> &'static Py<PyDict> {
        ADAPTERS.get().expect("adapters not initialize")
    }

    fn user_function_exception() -> &'static PyAtomicRef<Option<PyBaseException>> {
        USER_FUNCTION_EXCEPTION
            .get()
            .expect("user function exception not initialize")
    }

    fn enable_traceback() -> &'static PyAtomic<bool> {
        ENABLE_TRACEBACK
            .get()
            .expect("enable traceback not initialize")
    }

    pub(super) fn setup_module(module: &PyObject, vm: &VirtualMachine) {
        for (name, code) in ERROR_CODES {
            let name = vm.ctx.new_str(*name);
            let code = vm.new_pyobj(*code);
            module.set_attr(name, code, vm).unwrap();
        }

        setup_module_exceptions(module, vm);

        let _ = CONVERTERS.set(vm.ctx.new_dict());
        let _ = ADAPTERS.set(vm.ctx.new_dict());
        let _ = USER_FUNCTION_EXCEPTION.set(PyAtomicRef::from(None));
        let _ = ENABLE_TRACEBACK.set(Radium::new(false));

        module
            .set_attr("converters", converters().to_owned(), vm)
            .unwrap();
        module
            .set_attr("adapters", adapters().to_owned(), vm)
            .unwrap();
    }

    #[pyattr]
    #[pyclass(name)]
    #[derive(PyPayload)]
    struct Connection {
        db: PyMutex<Option<Sqlite>>,
        detect_types: c_int,
        isolation_level: PyAtomicRef<Option<PyStr>>,
        check_same_thread: bool,
        thread_ident: ThreadId,
        row_factory: PyAtomicRef<Option<PyObject>>,
        text_factory: PyAtomicRef<PyObject>,
    }

    impl Debug for Connection {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
            write!(f, "Sqlite3 Connection")
        }
    }

    impl Constructor for Connection {
        type Args = ConnectArgs;

        fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            Self::new(args, vm).and_then(|x| x.into_ref_with_type(vm, cls).map(Into::into))
        }
    }

    impl Callable for Connection {
        type Args = (PyStrRef,);

        fn call(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            if let Some(stmt) = Statement::new(zelf, &args.0, vm)? {
                Ok(stmt.into_ref(vm).into())
            } else {
                Ok(vm.ctx.none())
            }
        }
    }

    #[pyclass(with(Constructor, Callable), flags(BASETYPE))]
    impl Connection {
        fn new(args: ConnectArgs, vm: &VirtualMachine) -> PyResult<Self> {
            let path = args.database.into_cstring(vm)?;
            let db = Sqlite::from(SqliteRaw::open(path.as_ptr(), args.uri, vm)?);
            let timeout = (args.timeout * 1000.0) as c_int;
            db.busy_timeout(timeout);
            if let Some(isolation_level) = &args.isolation_level {
                begin_statement_ptr_from_isolation_level(isolation_level, vm)?;
            }
            let text_factory = PyStr::class(vm).to_owned().into_object();

            Ok(Self {
                db: PyMutex::new(Some(db)),
                detect_types: args.detect_types,
                isolation_level: PyAtomicRef::from(args.isolation_level),
                check_same_thread: args.check_same_thread,
                thread_ident: std::thread::current().id(),
                row_factory: PyAtomicRef::from(None),
                text_factory: PyAtomicRef::from(text_factory),
            })
        }

        fn db_lock(&self, vm: &VirtualMachine) -> PyResult<PyMappedMutexGuard<Sqlite>> {
            self.check_thread(vm)?;
            self._db_lock(vm)
        }

        fn _db_lock(&self, vm: &VirtualMachine) -> PyResult<PyMappedMutexGuard<Sqlite>> {
            let guard = self.db.lock();
            if guard.is_some() {
                Ok(PyMutexGuard::map(guard, |x| unsafe {
                    x.as_mut().unwrap_unchecked()
                }))
            } else {
                Err(new_programming_error(
                    vm,
                    "Cannot operate on a closed database.".to_owned(),
                ))
            }
        }

        #[pymethod]
        fn cursor(
            zelf: PyRef<Self>,
            factory: OptionalArg<ArgCallable>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Cursor>> {
            zelf.db_lock(vm).map(drop)?;

            let cursor = if let OptionalArg::Present(factory) = factory {
                let cursor = factory.invoke((zelf.clone(),), vm)?;
                let cursor = cursor.downcast::<Cursor>().map_err(|x| {
                    vm.new_type_error(format!("factory must return a cursor, not {}", x.class()))
                })?;
                unsafe { cursor.row_factory.swap(zelf.row_factory.to_owned()) };
                cursor
            } else {
                Cursor::new(zelf.clone(), zelf.row_factory.to_owned(), vm).into_ref(vm)
            };
            Ok(cursor)
        }

        #[pymethod]
        fn blobopen(
            zelf: PyRef<Self>,
            args: BlobOpenArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Blob>> {
            let table = args.table.to_cstring(vm)?;
            let column = args.column.to_cstring(vm)?;
            let name = args.name.to_cstring(vm)?;

            let db = zelf.db_lock(vm)?;

            let mut blob = null_mut();
            let ret = unsafe {
                sqlite3_blob_open(
                    db.db,
                    name.as_ptr(),
                    table.as_ptr(),
                    column.as_ptr(),
                    args.row,
                    (!args.readonly) as c_int,
                    &mut blob,
                )
            };
            db.check(ret, vm)?;
            drop(db);

            let blob = SqliteBlob { blob };
            let blob = Blob {
                connection: zelf,
                inner: PyMutex::new(Some(BlobInner { blob, offset: 0 })),
            };
            Ok(blob.into_ref(vm))
        }

        #[pymethod]
        fn close(&self, vm: &VirtualMachine) -> PyResult<()> {
            self.check_thread(vm)?;
            self.db.lock().take();
            Ok(())
        }

        #[pymethod]
        fn commit(&self, vm: &VirtualMachine) -> PyResult<()> {
            self.db_lock(vm)?.implicity_commit(vm)
        }

        #[pymethod]
        fn rollback(&self, vm: &VirtualMachine) -> PyResult<()> {
            let db = self.db_lock(vm)?;
            if !db.is_autocommit() {
                db._exec(b"ROLLBACK\0", vm)
            } else {
                Ok(())
            }
        }

        #[pymethod]
        fn execute(
            zelf: PyRef<Self>,
            sql: PyStrRef,
            parameters: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Cursor>> {
            let cursor = Cursor::new(zelf.clone(), zelf.row_factory.to_owned(), vm).into_ref(vm);
            Cursor::execute(cursor, sql, parameters, vm)
        }

        #[pymethod]
        fn executemany(
            zelf: PyRef<Self>,
            sql: PyStrRef,
            seq_of_params: ArgIterable,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Cursor>> {
            let cursor = Cursor::new(zelf.clone(), zelf.row_factory.to_owned(), vm).into_ref(vm);
            Cursor::executemany(cursor, sql, seq_of_params, vm)
        }

        #[pymethod]
        fn executescript(
            zelf: PyRef<Self>,
            script: PyStrRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Cursor>> {
            Cursor::executescript(
                Cursor::new(zelf.clone(), zelf.row_factory.to_owned(), vm).into_ref(vm),
                script,
                vm,
            )
        }

        #[pymethod]
        fn backup(zelf: PyRef<Self>, args: BackupArgs, vm: &VirtualMachine) -> PyResult<()> {
            let BackupArgs {
                target,
                pages,
                progress,
                name,
                sleep,
            } = args;
            if zelf.is(&target) {
                return Err(
                    vm.new_value_error("target cannot be the same connection instance".to_owned())
                );
            }

            let pages = if pages == 0 { -1 } else { pages };

            let name_cstring;
            let name_ptr = if let Some(name) = &name {
                name_cstring = name.to_cstring(vm)?;
                name_cstring.as_ptr()
            } else {
                b"main\0".as_ptr().cast()
            };

            let sleep_ms = (sleep * 1000.0) as c_int;

            let db = zelf.db_lock(vm)?;
            let target_db = target.db_lock(vm)?;

            let handle = unsafe {
                sqlite3_backup_init(target_db.db, b"main\0".as_ptr().cast(), db.db, name_ptr)
            };

            if handle.is_null() {
                return Err(target_db.error_extended(vm));
            }

            drop(db);
            drop(target_db);

            loop {
                let ret = unsafe { sqlite3_backup_step(handle, pages) };

                if let Some(progress) = &progress {
                    let remaining = unsafe { sqlite3_backup_remaining(handle) };
                    let pagecount = unsafe { sqlite3_backup_pagecount(handle) };
                    if let Err(err) = progress.invoke((ret, remaining, pagecount), vm) {
                        unsafe { sqlite3_backup_finish(handle) };
                        return Err(err);
                    }
                }

                if ret == SQLITE_BUSY || ret == SQLITE_LOCKED {
                    unsafe { sqlite3_sleep(sleep_ms) };
                } else if ret != SQLITE_OK {
                    break;
                }
            }

            let ret = unsafe { sqlite3_backup_finish(handle) };
            if ret == SQLITE_OK {
                Ok(())
            } else {
                Err(target.db_lock(vm)?.error_extended(vm))
            }
        }

        #[pymethod]
        fn create_function(&self, args: CreateFunctionArgs, vm: &VirtualMachine) -> PyResult<()> {
            let name = args.name.to_cstring(vm)?;
            let flags = if args.deterministic {
                SQLITE_UTF8 | SQLITE_DETERMINISTIC
            } else {
                SQLITE_UTF8
            };
            let db = self.db_lock(vm)?;
            let Some(data) = CallbackData::new(args.func, vm) else {
                return db.create_function(name.as_ptr(), args.narg, flags, null_mut(), None, None, None, None, vm);
            };

            db.create_function(
                name.as_ptr(),
                args.narg,
                flags,
                Box::into_raw(Box::new(data)).cast(),
                Some(CallbackData::func_callback),
                None,
                None,
                Some(CallbackData::destructor),
                vm,
            )
        }

        #[pymethod]
        fn create_aggregate(&self, args: CreateAggregateArgs, vm: &VirtualMachine) -> PyResult<()> {
            let name = args.name.to_cstring(vm)?;
            let db = self.db_lock(vm)?;
            let Some(data) = CallbackData::new(args.aggregate_class, vm) else {
                return db.create_function(name.as_ptr(), args.narg, SQLITE_UTF8, null_mut(), None, None, None, None, vm);
            };

            db.create_function(
                name.as_ptr(),
                args.narg,
                SQLITE_UTF8,
                Box::into_raw(Box::new(data)).cast(),
                None,
                Some(CallbackData::step_callback),
                Some(CallbackData::finalize_callback),
                Some(CallbackData::destructor),
                vm,
            )
        }

        #[pymethod]
        fn create_collation(
            &self,
            name: PyStrRef,
            callable: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let name = name.to_cstring(vm)?;
            let db = self.db_lock(vm)?;
            let Some(data )= CallbackData::new(callable, vm) else {
                unsafe {
                    sqlite3_create_collation_v2(db.db, name.as_ptr(), SQLITE_UTF8, null_mut(), None, None);
                }
                return Ok(());
            };
            let data = Box::into_raw(Box::new(data));

            let ret = unsafe {
                sqlite3_create_collation_v2(
                    db.db,
                    name.as_ptr(),
                    SQLITE_UTF8,
                    data.cast(),
                    Some(CallbackData::collation_callback),
                    Some(CallbackData::destructor),
                )
            };

            // TODO: replace with Result.inspect_err when stable
            if let Err(exc) = db.check(ret, vm) {
                // create_colation do not call destructor if error occur
                unsafe { Box::from_raw(data) };
                Err(exc)
            } else {
                Ok(())
            }
        }

        #[pymethod]
        fn create_window_function(
            &self,
            name: PyStrRef,
            narg: c_int,
            aggregate_class: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let name = name.to_cstring(vm)?;
            let db = self.db_lock(vm)?;
            let Some(data) = CallbackData::new(aggregate_class, vm) else {
                unsafe {
                    sqlite3_create_window_function(db.db, name.as_ptr(), narg, SQLITE_UTF8, null_mut(), None, None, None, None, None)
                };
                return Ok(());
            };

            let ret = unsafe {
                sqlite3_create_window_function(
                    db.db,
                    name.as_ptr(),
                    narg,
                    SQLITE_UTF8,
                    Box::into_raw(Box::new(data)).cast(),
                    Some(CallbackData::step_callback),
                    Some(CallbackData::finalize_callback),
                    Some(CallbackData::value_callback),
                    Some(CallbackData::inverse_callback),
                    Some(CallbackData::destructor),
                )
            };
            db.check(ret, vm)
                .map_err(|_| new_programming_error(vm, "Error creating window function".to_owned()))
        }

        #[pymethod]
        fn set_authorizer(&self, callable: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let db = self.db_lock(vm)?;
            let Some(data) = CallbackData::new(callable, vm) else {
                unsafe { sqlite3_set_authorizer(db.db, None, null_mut()) };
                return Ok(());
            };

            let ret = unsafe {
                sqlite3_set_authorizer(
                    db.db,
                    Some(CallbackData::authorizer_callback),
                    Box::into_raw(Box::new(data)).cast(),
                )
            };
            db.check(ret, vm).map_err(|_| {
                new_operational_error(vm, "Error setting authorizer callback".to_owned())
            })
        }

        #[pymethod]
        fn set_trace_callback(&self, callable: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let db = self.db_lock(vm)?;
            let Some(data )= CallbackData::new(callable, vm) else {
                unsafe {
                    sqlite3_trace_v2(db.db, SQLITE_TRACE_STMT as u32, None, null_mut())
                };
                return Ok(());
            };

            let ret = unsafe {
                sqlite3_trace_v2(
                    db.db,
                    SQLITE_TRACE_STMT as u32,
                    Some(CallbackData::trace_callback),
                    Box::into_raw(Box::new(data)).cast(),
                )
            };

            db.check(ret, vm)
        }

        #[pymethod]
        fn set_progress_handler(
            &self,
            callable: PyObjectRef,
            n: c_int,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let db = self.db_lock(vm)?;
            let Some(data )= CallbackData::new(callable, vm) else {
                unsafe { sqlite3_progress_handler(db.db, n, None, null_mut()) };
                return Ok(());
            };

            unsafe {
                sqlite3_progress_handler(
                    db.db,
                    n,
                    Some(CallbackData::progress_callback),
                    Box::into_raw(Box::new(data)).cast(),
                )
            };

            Ok(())
        }

        #[pymethod]
        fn iterdump(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let module = vm.import("sqlite3.dump", None, 0)?;
            let func = module.get_attr("_iterdump", vm)?;
            vm.invoke(&func, (zelf,))
        }

        #[pymethod]
        fn interrupt(&self, vm: &VirtualMachine) -> PyResult<()> {
            // DO NOT check thread safety
            self._db_lock(vm).map(|x| x.interrupt())
        }

        #[pymethod]
        fn getlimit(&self, category: c_int, vm: &VirtualMachine) -> PyResult<c_int> {
            self.db_lock(vm)?.limit(category, -1, vm)
        }

        #[pymethod]
        fn setlimit(&self, category: c_int, limit: c_int, vm: &VirtualMachine) -> PyResult<c_int> {
            self.db_lock(vm)?.limit(category, limit, vm)
        }

        #[pymethod(magic)]
        fn enter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pymethod(magic)]
        fn exit(
            &self,
            cls: PyObjectRef,
            exc: PyObjectRef,
            tb: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if vm.is_none(&cls) && vm.is_none(&exc) && vm.is_none(&tb) {
                self.commit(vm)
            } else {
                self.rollback(vm)
            }
        }

        #[pygetset]
        fn isolation_level(&self) -> Option<PyStrRef> {
            self.isolation_level.deref().map(|x| x.to_owned())
        }
        #[pygetset(setter)]
        fn set_isolation_level(&self, val: Option<PyStrRef>, vm: &VirtualMachine) -> PyResult<()> {
            if let Some(val) = &val {
                begin_statement_ptr_from_isolation_level(val, vm)?;
            }
            unsafe { self.isolation_level.swap(val) };
            Ok(())
        }

        #[pygetset]
        fn text_factory(&self) -> PyObjectRef {
            self.text_factory.to_owned()
        }
        #[pygetset(setter)]
        fn set_text_factory(&self, val: PyObjectRef) {
            unsafe { self.text_factory.swap(val) };
        }

        #[pygetset]
        fn row_factory(&self) -> Option<PyObjectRef> {
            self.row_factory.to_owned()
        }
        #[pygetset(setter)]
        fn set_row_factory(&self, val: Option<PyObjectRef>) {
            unsafe { self.row_factory.swap(val) };
        }

        fn check_thread(&self, vm: &VirtualMachine) -> PyResult<()> {
            if self.check_same_thread && (std::thread::current().id() != self.thread_ident) {
                Err(new_programming_error(
                    vm,
                    "SQLite objects created in a thread can only be used in that same thread."
                        .to_owned(),
                ))
            } else {
                Ok(())
            }
        }

        #[pygetset]
        fn in_transaction(&self, vm: &VirtualMachine) -> PyResult<bool> {
            self._db_lock(vm).map(|x| !x.is_autocommit())
        }

        #[pygetset]
        fn total_changes(&self, vm: &VirtualMachine) -> PyResult<c_int> {
            self._db_lock(vm).map(|x| x.total_changes())
        }
    }

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug, PyPayload)]
    struct Cursor {
        connection: PyRef<Connection>,
        arraysize: PyAtomic<c_int>,
        row_factory: PyAtomicRef<Option<PyObject>>,
        inner: PyMutex<Option<CursorInner>>,
    }

    #[derive(Debug)]
    struct CursorInner {
        description: Option<PyTupleRef>,
        row_cast_map: Vec<Option<PyObjectRef>>,
        lastrowid: i64,
        rowcount: i64,
        statement: Option<PyRef<Statement>>,
    }

    #[pyclass(with(Constructor, IterNext), flags(BASETYPE))]
    impl Cursor {
        fn new(
            connection: PyRef<Connection>,
            row_factory: Option<PyObjectRef>,
            _vm: &VirtualMachine,
        ) -> Self {
            Self {
                connection,
                arraysize: Radium::new(1),
                row_factory: PyAtomicRef::from(row_factory),
                inner: PyMutex::from(Some(CursorInner {
                    description: None,
                    row_cast_map: vec![],
                    lastrowid: -1,
                    rowcount: -1,
                    statement: None,
                })),
            }
        }

        fn inner(&self, vm: &VirtualMachine) -> PyResult<PyMappedMutexGuard<CursorInner>> {
            let guard = self.inner.lock();
            if guard.is_some() {
                Ok(PyMutexGuard::map(guard, |x| unsafe {
                    x.as_mut().unwrap_unchecked()
                }))
            } else {
                Err(new_programming_error(
                    vm,
                    "Cannot operate on a closed cursor.".to_owned(),
                ))
            }
        }

        #[pymethod]
        fn execute(
            zelf: PyRef<Self>,
            sql: PyStrRef,
            parameters: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let mut inner = zelf.inner(vm)?;

            if let Some(stmt) = inner.statement.take() {
                stmt.lock().reset();
            }

            let Some(stmt) = Statement::new(&zelf.connection, &sql, vm)? else {
                drop(inner);
                return Ok(zelf);
            };
            let stmt = stmt.into_ref(vm);

            inner.rowcount = if stmt.is_dml { 0 } else { -1 };

            let db = zelf.connection.db_lock(vm)?;

            if stmt.is_dml && db.is_autocommit() {
                db.begin_transaction(
                    zelf.connection
                        .isolation_level
                        .deref()
                        .map(|x| x.to_owned()),
                    vm,
                )?;
            }

            let st = stmt.lock();
            if let OptionalArg::Present(parameters) = parameters {
                st.bind_parameters(&parameters, vm)?;
            }

            let ret = st.step();

            if ret != SQLITE_DONE && ret != SQLITE_ROW {
                if let Some(exc) = unsafe { user_function_exception().swap(None) } {
                    return Err(exc);
                }
                return Err(db.error_extended(vm));
            }

            inner.row_cast_map = zelf.build_row_cast_map(&st, vm)?;

            inner.description = st.columns_description(vm)?;

            if ret == SQLITE_ROW {
                drop(st);
                inner.statement = Some(stmt);
            } else {
                st.reset();
                drop(st);
                if stmt.is_dml {
                    inner.rowcount += db.changes() as i64;
                }
            }

            inner.lastrowid = db.lastrowid();

            drop(inner);
            drop(db);
            Ok(zelf)
        }

        #[pymethod]
        fn executemany(
            zelf: PyRef<Self>,
            sql: PyStrRef,
            seq_of_params: ArgIterable,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let mut inner = zelf.inner(vm)?;

            if let Some(stmt) = inner.statement.take() {
                stmt.lock().reset();
            }

            let Some(stmt) = Statement::new(&zelf.connection, &sql, vm)? else {
                drop(inner);
                return Ok(zelf);
            };
            let stmt = stmt.into_ref(vm);

            let st = stmt.lock();

            if st.readonly() {
                return Err(new_programming_error(
                    vm,
                    "executemany() can only execute DML statements.".to_owned(),
                ));
            }

            inner.description = st.columns_description(vm)?;

            inner.rowcount = if stmt.is_dml { 0 } else { -1 };

            let db = zelf.connection.db_lock(vm)?;

            if stmt.is_dml && db.is_autocommit() {
                db.begin_transaction(
                    zelf.connection
                        .isolation_level
                        .deref()
                        .map(|x| x.to_owned()),
                    vm,
                )?;
            }

            let iter = seq_of_params.iter(vm)?;
            for params in iter {
                let params = params?;
                st.bind_parameters(&params, vm)?;

                if !st.step_row_else_done(vm)? {
                    if stmt.is_dml {
                        inner.rowcount += db.changes() as i64;
                    }
                    st.reset();
                }

                // if let Some(exc) = unsafe { user_function_exception().swap(None) } {
                //     return Err(exc);
                // }
            }

            if st.busy() {
                drop(st);
                inner.statement = Some(stmt);
            }

            drop(inner);
            drop(db);
            Ok(zelf)
        }

        #[pymethod]
        fn executescript(
            zelf: PyRef<Self>,
            script: PyStrRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let db = zelf.connection.db_lock(vm)?;

            db.sql_limit(script.byte_len(), vm)?;

            db.implicity_commit(vm)?;

            let script = script.to_cstring(vm)?;
            let mut ptr = script.as_ptr();

            while let Some(st) = db.prepare(ptr, &mut ptr, vm)? {
                while st.step_row_else_done(vm)? {}
            }

            drop(db);
            Ok(zelf)
        }

        #[pymethod]
        fn fetchone(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Self::next(&zelf, vm).map(|x| match x {
                PyIterReturn::Return(row) => row,
                PyIterReturn::StopIteration(_) => vm.ctx.none(),
            })
        }

        #[pymethod]
        fn fetchmany(
            zelf: PyRef<Self>,
            max_rows: OptionalArg<c_int>,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<PyObjectRef>> {
            let max_rows = max_rows.unwrap_or_else(|| zelf.arraysize.load(Ordering::Relaxed));
            let mut list = vec![];
            while let PyIterReturn::Return(row) = Self::next(&zelf, vm)? {
                list.push(row);
                if list.len() as c_int >= max_rows {
                    break;
                }
            }
            Ok(list)
        }

        #[pymethod]
        fn fetchall(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            let mut list = vec![];
            while let PyIterReturn::Return(row) = Self::next(&zelf, vm)? {
                list.push(row);
            }
            Ok(list)
        }

        #[pymethod]
        fn close(&self) {
            if let Some(inner) = self.inner.lock().take() {
                if let Some(stmt) = inner.statement {
                    stmt.lock().reset();
                }
            }
        }

        #[pymethod]
        fn setinputsizes(&self, _sizes: PyObjectRef) {}
        #[pymethod]
        fn setoutputsize(&self, _size: PyObjectRef, _column: OptionalArg<PyObjectRef>) {}

        #[pygetset]
        fn connection(&self) -> PyRef<Connection> {
            self.connection.clone()
        }

        #[pygetset]
        fn lastrowid(&self, vm: &VirtualMachine) -> PyResult<i64> {
            self.inner(vm).map(|x| x.lastrowid)
        }

        #[pygetset]
        fn rowcount(&self, vm: &VirtualMachine) -> PyResult<i64> {
            self.inner(vm).map(|x| x.rowcount)
        }

        #[pygetset]
        fn description(&self, vm: &VirtualMachine) -> PyResult<Option<PyTupleRef>> {
            self.inner(vm).map(|x| x.description.clone())
        }

        #[pygetset]
        fn arraysize(&self) -> c_int {
            self.arraysize.load(Ordering::Relaxed)
        }
        #[pygetset(setter)]
        fn set_arraysize(&self, val: c_int) {
            self.arraysize.store(val, Ordering::Relaxed);
        }

        fn build_row_cast_map(
            &self,
            st: &SqliteStatementRaw,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<Option<PyObjectRef>>> {
            if self.connection.detect_types == 0 {
                return Ok(vec![]);
            }

            let mut cast_map = vec![];
            let num_cols = st.column_count();

            for i in 0..num_cols {
                if self.connection.detect_types & PARSE_COLNAMES != 0 {
                    let col_name = st.column_name(i);
                    let col_name = ptr_to_str(col_name, vm)?;
                    let col_name = col_name
                        .chars()
                        .skip_while(|&x| x != '[')
                        .skip(1)
                        .take_while(|&x| x != ']')
                        .flat_map(|x| x.to_uppercase())
                        .collect::<String>();
                    if let Some(converter) = converters().get_item_opt(&col_name, vm)? {
                        cast_map.push(Some(converter.clone()));
                        continue;
                    }
                }
                if self.connection.detect_types & PARSE_DECLTYPES != 0 {
                    let decltype = st.column_decltype(i);
                    let decltype = ptr_to_str(decltype, vm)?;
                    if let Some(decltype) = decltype.split_terminator(&[' ', '(']).next() {
                        let decltype = decltype.to_uppercase();
                        if let Some(converter) = converters().get_item_opt(&decltype, vm)? {
                            cast_map.push(Some(converter.clone()));
                            continue;
                        }
                    }
                }
                cast_map.push(None);
            }

            Ok(cast_map)
        }
    }

    impl Constructor for Cursor {
        type Args = (PyRef<Connection>,);

        fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            Self::new(args.0, None, vm)
                .into_ref_with_type(vm, cls)
                .map(Into::into)
        }
    }

    impl IterNextIterable for Cursor {}
    impl IterNext for Cursor {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let mut inner = zelf.inner(vm)?;
            let Some(stmt) = &inner.statement else {
                return Ok(PyIterReturn::StopIteration(None));
            };
            let st = stmt.lock();
            let db = zelf.connection.db_lock(vm)?;
            // fetch_one_row

            let num_cols = st.data_count();

            let mut row = Vec::with_capacity(num_cols as usize);

            for i in 0..num_cols {
                let val = if let Some(converter) =
                    inner.row_cast_map.get(i as usize).cloned().flatten()
                {
                    let blob = st.column_blob(i);
                    if blob.is_null() {
                        vm.ctx.none()
                    } else {
                        let nbytes = st.column_bytes(i);
                        let blob = unsafe {
                            std::slice::from_raw_parts(blob.cast::<u8>(), nbytes as usize)
                        };
                        let blob = vm.ctx.new_bytes(blob.to_vec());
                        vm.invoke(&converter, (blob,))?
                    }
                } else {
                    let col_type = st.column_type(i);
                    match col_type {
                        SQLITE_NULL => vm.ctx.none(),
                        SQLITE_INTEGER => vm.ctx.new_int(st.column_int(i)).into(),
                        SQLITE_FLOAT => vm.ctx.new_float(st.column_double(i)).into(),
                        SQLITE_TEXT => {
                            let text =
                                ptr_to_vec(st.column_text(i), st.column_bytes(i), db.db, vm)?;

                            let text_factory = zelf.connection.text_factory.to_owned();

                            if text_factory.is(PyStr::class(vm)) {
                                let text = String::from_utf8(text).map_err(|_| {
                                    new_operational_error(vm, "not valid UTF-8".to_owned())
                                })?;
                                vm.ctx.new_str(text).into()
                            } else if text_factory.is(PyBytes::class(vm)) {
                                vm.ctx.new_bytes(text).into()
                            } else if text_factory.is(PyByteArray::class(vm)) {
                                PyByteArray::from(text).into_ref(vm).into()
                            } else {
                                let bytes = vm.ctx.new_bytes(text);
                                vm.invoke(&text_factory, (bytes,))?
                            }
                        }
                        SQLITE_BLOB => {
                            let blob = ptr_to_vec(
                                st.column_blob(i).cast(),
                                st.column_bytes(i),
                                db.db,
                                vm,
                            )?;

                            vm.ctx.new_bytes(blob).into()
                        }
                        _ => {
                            return Err(vm.new_not_implemented_error(format!(
                                "unknown column type: {col_type}"
                            )));
                        }
                    }
                };

                row.push(val);
            }

            if !st.step_row_else_done(vm)? {
                st.reset();
                drop(st);
                if stmt.is_dml {
                    inner.rowcount = db.changes() as i64;
                }
                inner.statement = None;
            } else {
                drop(st);
            }

            drop(db);
            drop(inner);

            let row = vm.ctx.new_tuple(row);

            if let Some(row_factory) = zelf.row_factory.to_owned() {
                vm.invoke(&row_factory, (zelf.to_owned(), row))
                    .map(PyIterReturn::Return)
            } else {
                Ok(PyIterReturn::Return(row.into()))
            }
        }
    }

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug, PyPayload)]
    struct Row {
        data: PyTupleRef,
        description: PyTupleRef,
    }

    #[pyclass(
        with(Constructor, Hashable, Comparable, Iterable, AsMapping, AsSequence),
        flags(BASETYPE)
    )]
    impl Row {
        #[pymethod]
        fn keys(&self, _vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            Ok(self
                .description
                .iter()
                .map(|x| x.payload::<PyTuple>().unwrap().as_slice()[0].clone())
                .collect())
        }

        fn subscript(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
            if let Some(i) = needle.payload::<PyInt>() {
                let i = i.try_to_primitive::<isize>(vm)?;
                self.data.getitem_by_index(vm, i)
            } else if let Some(name) = needle.payload::<PyStr>() {
                for (obj, i) in self.description.iter().zip(0..) {
                    let obj = &obj.payload::<PyTuple>().unwrap().as_slice()[0];
                    let Some(obj) = obj.payload::<PyStr>() else { break; };
                    let a_iter = name.as_str().chars().flat_map(|x| x.to_uppercase());
                    let b_iter = obj.as_str().chars().flat_map(|x| x.to_uppercase());

                    if a_iter.eq(b_iter) {
                        return self.data.getitem_by_index(vm, i);
                    }
                }
                Err(vm.new_index_error("No item with that key".to_owned()))
            } else if let Some(slice) = needle.payload::<PySlice>() {
                let list = self.data.getitem_by_slice(vm, slice.to_saturated(vm)?)?;
                Ok(vm.ctx.new_tuple(list).into())
            } else {
                Err(vm.new_index_error("Index must be int or string".to_owned()))
            }
        }
    }

    impl Constructor for Row {
        type Args = (PyRef<Cursor>, PyTupleRef);

        fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            let description = args
                .0
                .inner(vm)?
                .description
                .clone()
                .ok_or_else(|| vm.new_value_error("no description in Cursor".to_owned()))?;

            Self {
                data: args.1,
                description,
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    impl Hashable for Row {
        fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
            Ok(zelf.description.as_object().hash(vm)? | zelf.data.as_object().hash(vm)?)
        }
    }

    impl Comparable for Row {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            op.eq_only(|| {
                if let Some(other) = other.payload::<Self>() {
                    let eq = vm
                        .bool_eq(zelf.description.as_object(), other.description.as_object())?
                        && vm.bool_eq(zelf.data.as_object(), other.data.as_object())?;
                    Ok(eq.into())
                } else {
                    Ok(PyComparisonValue::NotImplemented)
                }
            })
        }
    }

    impl Iterable for Row {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Iterable::iter(zelf.data.clone(), vm)
        }
    }

    impl AsMapping for Row {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: once_cell::sync::Lazy<PyMappingMethods> =
                once_cell::sync::Lazy::new(|| PyMappingMethods {
                    length: atomic_func!(|mapping, _vm| Ok(Row::mapping_downcast(mapping)
                        .data
                        .len())),
                    subscript: atomic_func!(|mapping, needle, vm| {
                        Row::mapping_downcast(mapping).subscript(needle, vm)
                    }),
                    ..PyMappingMethods::NOT_IMPLEMENTED
                });
            &AS_MAPPING
        }
    }

    impl AsSequence for Row {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: once_cell::sync::Lazy<PySequenceMethods> =
                once_cell::sync::Lazy::new(|| PySequenceMethods {
                    length: atomic_func!(|seq, _vm| Ok(Row::sequence_downcast(seq).data.len())),
                    item: atomic_func!(|seq, i, vm| Row::sequence_downcast(seq)
                        .data
                        .getitem_by_index(vm, i)),
                    ..PySequenceMethods::NOT_IMPLEMENTED
                });
            &AS_SEQUENCE
        }
    }

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug, PyPayload)]
    struct Blob {
        connection: PyRef<Connection>,
        inner: PyMutex<Option<BlobInner>>,
    }

    #[derive(Debug)]
    struct BlobInner {
        blob: SqliteBlob,
        offset: c_int,
    }

    impl Drop for BlobInner {
        fn drop(&mut self) {
            unsafe { sqlite3_blob_close(self.blob.blob) };
        }
    }

    #[pyclass(with(AsMapping))]
    impl Blob {
        #[pymethod]
        fn close(&self) {
            self.inner.lock().take();
        }

        #[pymethod]
        fn read(
            &self,
            length: OptionalArg<c_int>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyBytes>> {
            let mut length = length.unwrap_or(-1);
            let mut inner = self.inner(vm)?;
            let blob_len = inner.blob.bytes();
            let max_read = blob_len - inner.offset;

            if length < 0 || length > max_read {
                length = max_read;
            }

            if length == 0 {
                Ok(vm.ctx.empty_bytes.clone())
            } else {
                let mut buf = Vec::<u8>::with_capacity(length as usize);
                let ret = inner
                    .blob
                    .read(buf.as_mut_ptr().cast(), length, inner.offset);
                self.check(ret, vm)?;
                unsafe { buf.set_len(length as usize) };
                inner.offset += length;
                Ok(vm.ctx.new_bytes(buf))
            }
        }

        #[pymethod]
        fn write(&self, data: PyBuffer, vm: &VirtualMachine) -> PyResult<()> {
            let mut inner = self.inner(vm)?;
            let blob_len = inner.blob.bytes();
            let length = Self::expect_write(blob_len, data.desc.len, inner.offset, vm)?;

            let ret = data.contiguous_or_collect(|buf| {
                inner.blob.write(buf.as_ptr().cast(), length, inner.offset)
            });

            self.check(ret, vm)?;
            inner.offset += length;
            Ok(())
        }

        #[pymethod]
        fn tell(&self, vm: &VirtualMachine) -> PyResult<c_int> {
            self.inner(vm).map(|x| x.offset)
        }

        #[pymethod]
        fn seek(
            &self,
            mut offset: c_int,
            origin: OptionalArg<c_int>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let origin = origin.unwrap_or(libc::SEEK_SET);
            let mut inner = self.inner(vm)?;
            let blob_len = inner.blob.bytes();

            let overflow_err =
                || vm.new_overflow_error("seek offset results in overflow".to_owned());

            match origin {
                libc::SEEK_SET => {}
                libc::SEEK_CUR => {
                    offset = offset.checked_add(inner.offset).ok_or_else(overflow_err)?
                }
                libc::SEEK_END => offset = offset.checked_add(blob_len).ok_or_else(overflow_err)?,
                _ => {
                    return Err(vm.new_value_error(
                        "'origin' should be os.SEEK_SET, os.SEEK_CUR, or os.SEEK_END".to_owned(),
                    ))
                }
            }

            if offset < 0 || offset > blob_len {
                Err(vm.new_value_error("offset out of blob range".to_owned()))
            } else {
                inner.offset = offset;
                Ok(())
            }
        }

        #[pymethod(magic)]
        fn enter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pymethod(magic)]
        fn exit(&self, _args: FuncArgs) {
            self.close()
        }

        fn inner(&self, vm: &VirtualMachine) -> PyResult<PyMappedMutexGuard<BlobInner>> {
            let guard = self.inner.lock();
            if guard.is_some() {
                Ok(PyMutexGuard::map(guard, |x| unsafe {
                    x.as_mut().unwrap_unchecked()
                }))
            } else {
                Err(new_programming_error(
                    vm,
                    "Cannot operate on a closed blob.".to_owned(),
                ))
            }
        }

        fn wrapped_index(index: PyIntRef, length: c_int, vm: &VirtualMachine) -> PyResult<c_int> {
            let mut index = index.try_to_primitive::<c_int>(vm)?;
            if index < 0 {
                index += length;
            }
            if index < 0 || index >= length {
                Err(vm.new_index_error("Blob index out of range".to_owned()))
            } else {
                Ok(index)
            }
        }

        fn expect_write(
            blob_len: c_int,
            length: usize,
            offset: c_int,
            vm: &VirtualMachine,
        ) -> PyResult<c_int> {
            let max_write = blob_len - offset;
            if length <= max_write as usize {
                Ok(length as c_int)
            } else {
                Err(vm.new_value_error("data longer than blob length".to_owned()))
            }
        }

        fn subscript(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
            let inner = self.inner(vm)?;
            if let Some(index) = needle.try_index_opt(vm) {
                let blob_len = inner.blob.bytes();
                let index = Self::wrapped_index(index?, blob_len, vm)?;
                let mut byte: u8 = 0;
                let ret = inner.blob.read_single(&mut byte, index);
                self.check(ret, vm).map(|_| vm.ctx.new_int(byte).into())
            } else if let Some(slice) = needle.payload::<PySlice>() {
                let blob_len = inner.blob.bytes();
                let slice = slice.to_saturated(vm)?;
                let (range, step, length) = slice.adjust_indices(blob_len as usize);
                let mut buf = Vec::<u8>::with_capacity(length);

                if step == 1 {
                    let ret = inner.blob.read(
                        buf.as_mut_ptr().cast(),
                        length as c_int,
                        range.start as c_int,
                    );
                    self.check(ret, vm)?;
                    unsafe { buf.set_len(length) };
                } else {
                    let iter = SaturatedSliceIter::from_adjust_indices(range, step, length);
                    let mut byte: u8 = 0;
                    for index in iter {
                        let ret = inner.blob.read_single(&mut byte, index as c_int);
                        self.check(ret, vm)?;
                        buf.push(byte);
                    }
                }
                Ok(vm.ctx.new_bytes(buf).into())
            } else {
                Err(vm.new_type_error("Blob indices must be integers".to_owned()))
            }
        }

        fn ass_subscript(
            &self,
            needle: &PyObject,
            value: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let Some(value) = value else {
                return Err(vm.new_type_error("Blob doesn't support deletion".to_owned()));
            };
            let inner = self.inner(vm)?;

            if let Some(index) = needle.try_index_opt(vm) {
                let Some(value) = value.payload::<PyInt>() else {
                    return Err(vm.new_type_error(format!("'{}' object cannot be interpreted as an integer", value.class())));
                };
                let value = value.try_to_primitive::<u8>(vm)?;
                let blob_len = inner.blob.bytes();
                let index = Self::wrapped_index(index?, blob_len, vm)?;
                Self::expect_write(blob_len, 1, index, vm)?;
                let ret = inner.blob.write_single(value, index);
                self.check(ret, vm)
            } else if let Some(_slice) = needle.payload::<PySlice>() {
                Err(vm
                    .new_not_implemented_error("Blob slice assigment is not implmented".to_owned()))
                // let blob_len = inner.blob.bytes();
                // let slice = slice.to_saturated(vm)?;
                // let (range, step, length) = slice.adjust_indices(blob_len as usize);
            } else {
                Err(vm.new_type_error("Blob indices must be integers".to_owned()))
            }
        }

        fn check(&self, ret: c_int, vm: &VirtualMachine) -> PyResult<()> {
            if ret == SQLITE_OK {
                Ok(())
            } else {
                Err(self.connection.db_lock(vm)?.error_extended(vm))
            }
        }
    }

    impl AsMapping for Blob {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                length: atomic_func!(|mapping, vm| Blob::mapping_downcast(mapping)
                    .inner(vm)
                    .map(|x| x.blob.bytes() as usize)),
                subscript: atomic_func!(|mapping, needle, vm| {
                    Blob::mapping_downcast(mapping).subscript(needle, vm)
                }),
                ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                    Blob::mapping_downcast(mapping).ass_subscript(needle, value, vm)
                }),
            };
            &AS_MAPPING
        }
    }

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug, PyPayload)]
    struct PrepareProtocol {}

    #[pyclass()]
    impl PrepareProtocol {}

    #[pyattr]
    #[pyclass(name)]
    #[derive(PyPayload)]
    struct Statement {
        st: PyMutex<SqliteStatement>,
        pub is_dml: bool,
    }

    impl Debug for Statement {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
            write!(
                f,
                "{} Statement",
                if self.is_dml { "DML" } else { "Non-DML" }
            )
        }
    }

    #[pyclass()]
    impl Statement {
        fn new(
            connection: &Connection,
            sql: &PyStr,
            vm: &VirtualMachine,
        ) -> PyResult<Option<Self>> {
            let sql_cstr = sql.to_cstring(vm)?;
            let sql_len = sql.byte_len() + 1;

            let db = connection.db_lock(vm)?;

            db.sql_limit(sql_len, vm)?;

            let mut tail = null();
            let st = db.prepare(sql_cstr.as_ptr(), &mut tail, vm)?;

            let Some(st) = st else {
                return Ok(None);
            };

            let tail = unsafe { CStr::from_ptr(tail) };
            let tail = tail.to_bytes();
            if lstrip_sql(tail).is_some() {
                return Err(new_programming_error(
                    vm,
                    "You can only execute one statement at a time.".to_owned(),
                ));
            }

            let is_dml = if let Some(head) = lstrip_sql(sql_cstr.as_bytes()) {
                head.len() >= 6
                    && (head[..6].eq_ignore_ascii_case(b"insert")
                        || head[..6].eq_ignore_ascii_case(b"update")
                        || head[..6].eq_ignore_ascii_case(b"delete")
                        || (head.len() >= 7 && head[..7].eq_ignore_ascii_case(b"replace")))
            } else {
                false
            };

            Ok(Some(Self {
                st: PyMutex::from(st),
                is_dml,
            }))
        }

        fn lock(&self) -> PyMutexGuard<SqliteStatement> {
            self.st.lock()
        }
    }

    struct Sqlite {
        raw: SqliteRaw,
    }

    impl From<SqliteRaw> for Sqlite {
        fn from(raw: SqliteRaw) -> Self {
            Self { raw }
        }
    }

    impl Drop for Sqlite {
        fn drop(&mut self) {
            unsafe { sqlite3_close_v2(self.raw.db) };
        }
    }

    impl Deref for Sqlite {
        type Target = SqliteRaw;

        fn deref(&self) -> &Self::Target {
            &self.raw
        }
    }

    #[derive(Copy, Clone)]
    struct SqliteRaw {
        db: *mut sqlite3,
    }

    cfg_if::cfg_if! {
        if #[cfg(feature = "threading")] {
            unsafe impl Send for SqliteStatement {}
            // unsafe impl Sync for SqliteStatement {}
            unsafe impl Send for Sqlite {}
            // unsafe impl Sync for Sqlite {}
            unsafe impl Send for SqliteBlob {}
        }
    }

    impl From<SqliteStatementRaw> for SqliteRaw {
        fn from(stmt: SqliteStatementRaw) -> Self {
            unsafe {
                Self {
                    db: sqlite3_db_handle(stmt.st),
                }
            }
        }
    }

    impl SqliteRaw {
        fn check(self, ret: c_int, vm: &VirtualMachine) -> PyResult<()> {
            if ret == SQLITE_OK {
                Ok(())
            } else {
                Err(self.error_extended(vm))
            }
        }

        fn error_extended(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
            let errcode = unsafe { sqlite3_errcode(self.db) };
            let typ = exception_type_from_errcode(errcode, vm);
            let extended_errcode = unsafe { sqlite3_extended_errcode(self.db) };
            let errmsg = unsafe { sqlite3_errmsg(self.db) };
            let errmsg = unsafe { CStr::from_ptr(errmsg) };
            let errmsg = errmsg.to_str().unwrap().to_owned();

            raise_exception(typ.to_owned(), extended_errcode, errmsg, vm)
        }

        fn open(path: *const libc::c_char, uri: bool, vm: &VirtualMachine) -> PyResult<Self> {
            let mut db = null_mut();
            let ret = unsafe {
                sqlite3_open_v2(
                    path,
                    addr_of_mut!(db),
                    SQLITE_OPEN_READWRITE
                        | SQLITE_OPEN_CREATE
                        | if uri { SQLITE_OPEN_URI } else { 0 },
                    null(),
                )
            };
            let zelf = Self { db };
            zelf.check(ret, vm).map(|_| zelf)
        }

        fn _exec(self, sql: &[u8], vm: &VirtualMachine) -> PyResult<()> {
            let ret =
                unsafe { sqlite3_exec(self.db, sql.as_ptr().cast(), None, null_mut(), null_mut()) };
            self.check(ret, vm)
        }

        fn prepare(
            self,
            sql: *const libc::c_char,
            tail: *mut *const libc::c_char,
            vm: &VirtualMachine,
        ) -> PyResult<Option<SqliteStatement>> {
            let mut st = null_mut();
            let ret = unsafe { sqlite3_prepare_v2(self.db, sql, -1, &mut st, tail) };
            self.check(ret, vm)?;
            if st.is_null() {
                Ok(None)
            } else {
                Ok(Some(SqliteStatement::from(SqliteStatementRaw::from(st))))
            }
        }

        fn limit(self, category: c_int, limit: c_int, vm: &VirtualMachine) -> PyResult<c_int> {
            let old_limit = unsafe { sqlite3_limit(self.db, category, limit) };
            if old_limit >= 0 {
                Ok(old_limit)
            } else {
                Err(new_programming_error(
                    vm,
                    "'category' is out of bounds".to_owned(),
                ))
            }
        }

        fn sql_limit(self, len: usize, vm: &VirtualMachine) -> PyResult<()> {
            if len <= unsafe { sqlite3_limit(self.db, SQLITE_LIMIT_SQL_LENGTH, -1) } as usize {
                Ok(())
            } else {
                Err(new_data_error(vm, "query string is too large".to_owned()))
            }
        }

        fn is_autocommit(self) -> bool {
            unsafe { sqlite3_get_autocommit(self.db) != 0 }
        }

        fn changes(self) -> c_int {
            unsafe { sqlite3_changes(self.db) }
        }

        fn total_changes(self) -> c_int {
            unsafe { sqlite3_total_changes(self.db) }
        }

        fn lastrowid(self) -> c_longlong {
            unsafe { sqlite3_last_insert_rowid(self.db) }
        }

        fn implicity_commit(self, vm: &VirtualMachine) -> PyResult<()> {
            if self.is_autocommit() {
                Ok(())
            } else {
                self._exec(b"COMMIT\0", vm)
            }
        }

        fn begin_transaction(
            self,
            isolation_level: Option<PyStrRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let Some(isolation_level) = isolation_level else {
                return Ok(());
            };
            let mut s = Vec::with_capacity(16);
            s.extend(b"BEGIN ");
            s.extend(isolation_level.as_str().bytes());
            s.push(b'\0');
            self._exec(&s, vm)
        }

        fn interrupt(self) {
            unsafe { sqlite3_interrupt(self.db) }
        }

        fn busy_timeout(self, timeout: i32) {
            unsafe { sqlite3_busy_timeout(self.db, timeout) };
        }

        #[allow(clippy::too_many_arguments)]
        fn create_function(
            self,
            name: *const libc::c_char,
            narg: c_int,
            flags: c_int,
            data: *mut c_void,
            func: Option<
                unsafe extern "C" fn(
                    arg1: *mut sqlite3_context,
                    arg2: c_int,
                    arg3: *mut *mut sqlite3_value,
                ),
            >,
            step: Option<
                unsafe extern "C" fn(
                    arg1: *mut sqlite3_context,
                    arg2: c_int,
                    arg3: *mut *mut sqlite3_value,
                ),
            >,
            finalize: Option<unsafe extern "C" fn(arg1: *mut sqlite3_context)>,
            destroy: Option<unsafe extern "C" fn(arg1: *mut c_void)>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let ret = unsafe {
                sqlite3_create_function_v2(
                    self.db, name, narg, flags, data, func, step, finalize, destroy,
                )
            };
            self.check(ret, vm)
                .map_err(|_| new_operational_error(vm, "Error creating function".to_owned()))
        }
    }

    struct SqliteStatement {
        raw: SqliteStatementRaw,
    }

    impl From<SqliteStatementRaw> for SqliteStatement {
        fn from(raw: SqliteStatementRaw) -> Self {
            Self { raw }
        }
    }

    impl Drop for SqliteStatement {
        fn drop(&mut self) {
            unsafe {
                sqlite3_finalize(self.raw.st);
            }
        }
    }

    impl Deref for SqliteStatement {
        type Target = SqliteStatementRaw;

        fn deref(&self) -> &Self::Target {
            &self.raw
        }
    }

    #[derive(Copy, Clone)]
    struct SqliteStatementRaw {
        st: *mut sqlite3_stmt,
    }

    impl From<*mut sqlite3_stmt> for SqliteStatementRaw {
        fn from(st: *mut sqlite3_stmt) -> Self {
            SqliteStatementRaw { st }
        }
    }

    impl SqliteStatementRaw {
        fn step(self) -> c_int {
            unsafe { sqlite3_step(self.st) }
        }

        fn step_row_else_done(self, vm: &VirtualMachine) -> PyResult<bool> {
            let ret = self.step();

            if let Some(exc) = unsafe { user_function_exception().swap(None) } {
                Err(exc)
            } else if ret == SQLITE_ROW {
                Ok(true)
            } else if ret == SQLITE_DONE {
                Ok(false)
            } else {
                Err(SqliteRaw::from(self).error_extended(vm))
            }
        }

        fn reset(self) {
            unsafe { sqlite3_reset(self.st) };
        }

        fn data_count(self) -> c_int {
            unsafe { sqlite3_data_count(self.st) }
        }

        fn bind_parameter(
            self,
            pos: c_int,
            parameter: &PyObject,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let adapted;
            let obj = if need_adapt(parameter, vm) {
                adapted = _adapt(
                    parameter,
                    PrepareProtocol::class(vm).to_owned(),
                    |x| Ok(x.to_owned()),
                    vm,
                )?;
                &adapted
            } else {
                parameter
            };

            let ret = if vm.is_none(obj) {
                unsafe { sqlite3_bind_null(self.st, pos) }
            } else if let Some(val) = obj.payload::<PyInt>() {
                let val = val.try_to_primitive::<i64>(vm)?;
                unsafe { sqlite3_bind_int64(self.st, pos, val) }
            } else if let Some(val) = obj.payload::<PyFloat>() {
                let val = val.to_f64();
                unsafe { sqlite3_bind_double(self.st, pos, val) }
            } else if let Some(val) = obj.payload::<PyStr>() {
                let (ptr, len) = str_to_ptr_len(val, vm)?;
                unsafe { sqlite3_bind_text(self.st, pos, ptr, len, SQLITE_TRANSIENT()) }
            } else if let Ok(buffer) = PyBuffer::try_from_borrowed_object(vm, obj) {
                let (ptr, len) = buffer_to_ptr_len(&buffer, vm)?;
                unsafe { sqlite3_bind_blob(self.st, pos, ptr, len, SQLITE_TRANSIENT()) }
            } else {
                return Err(new_programming_error(
                    vm,
                    format!(
                        "Error binding parameter {}: type '{}' is not supported",
                        pos,
                        obj.class()
                    ),
                ));
            };

            if ret == SQLITE_OK {
                Ok(())
            } else {
                let db = SqliteRaw::from(self);
                db.check(ret, vm)
            }
        }

        fn bind_parameters(self, parameters: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            if let Some(dict) = parameters.downcast_ref::<PyDict>() {
                self.bind_parameters_name(dict, vm)
            } else if let Ok(seq) = PySequence::try_protocol(parameters, vm) {
                self.bind_parameters_sequence(seq, vm)
            } else {
                Err(new_programming_error(
                    vm,
                    "parameters are of unsupported type".to_owned(),
                ))
            }
        }

        fn bind_parameters_name(self, dict: &Py<PyDict>, vm: &VirtualMachine) -> PyResult<()> {
            let num_needed = unsafe { sqlite3_bind_parameter_count(self.st) };

            for i in 1..=num_needed {
                let name = unsafe { sqlite3_bind_parameter_name(self.st, i) };
                if name.is_null() {
                    return Err(new_programming_error(vm, "Binding {} has no name, but you supplied a dictionary (which has only names).".to_owned()));
                }
                let name = unsafe { name.add(1) };
                let name = ptr_to_str(name, vm)?;

                let val = dict.get_item(name, vm)?;

                self.bind_parameter(i, &val, vm)?;
            }
            Ok(())
        }

        fn bind_parameters_sequence(self, seq: PySequence, vm: &VirtualMachine) -> PyResult<()> {
            let num_needed = unsafe { sqlite3_bind_parameter_count(self.st) };
            if seq.length(vm)? != num_needed as usize {
                return Err(new_programming_error(
                    vm,
                    "Incorrect number of binding supplied".to_owned(),
                ));
            }

            for i in 1..=num_needed {
                let val = seq.get_item(i as isize - 1, vm)?;
                self.bind_parameter(i, &val, vm)?;
            }
            Ok(())
        }

        fn column_count(self) -> c_int {
            unsafe { sqlite3_column_count(self.st) }
        }

        fn column_type(self, pos: c_int) -> c_int {
            unsafe { sqlite3_column_type(self.st, pos) }
        }

        fn column_int(self, pos: c_int) -> i64 {
            unsafe { sqlite3_column_int64(self.st, pos) }
        }

        fn column_double(self, pos: c_int) -> f64 {
            unsafe { sqlite3_column_double(self.st, pos) }
        }

        fn column_blob(self, pos: c_int) -> *const c_void {
            unsafe { sqlite3_column_blob(self.st, pos) }
        }

        fn column_text(self, pos: c_int) -> *const u8 {
            unsafe { sqlite3_column_text(self.st, pos) }
        }

        fn column_decltype(self, pos: c_int) -> *const libc::c_char {
            unsafe { sqlite3_column_decltype(self.st, pos) }
        }

        fn column_bytes(self, pos: c_int) -> c_int {
            unsafe { sqlite3_column_bytes(self.st, pos) }
        }

        fn column_name(self, pos: c_int) -> *const libc::c_char {
            unsafe { sqlite3_column_name(self.st, pos) }
        }

        fn columns_name(self, vm: &VirtualMachine) -> PyResult<Vec<PyStrRef>> {
            let count = self.column_count();
            (0..count)
                .map(|i| {
                    let name = self.column_name(i);
                    ptr_to_str(name, vm).map(|x| vm.ctx.new_str(x))
                })
                .collect()
        }

        fn columns_description(self, vm: &VirtualMachine) -> PyResult<Option<PyTupleRef>> {
            if self.column_count() == 0 {
                return Ok(None);
            }
            let columns = self
                .columns_name(vm)?
                .into_iter()
                .map(|s| {
                    vm.ctx
                        .new_tuple(vec![
                            s.into(),
                            vm.ctx.none(),
                            vm.ctx.none(),
                            vm.ctx.none(),
                            vm.ctx.none(),
                            vm.ctx.none(),
                            vm.ctx.none(),
                        ])
                        .into()
                })
                .collect();
            Ok(Some(vm.ctx.new_tuple(columns)))
        }

        fn busy(self) -> bool {
            unsafe { sqlite3_stmt_busy(self.st) != 0 }
        }

        fn readonly(self) -> bool {
            unsafe { sqlite3_stmt_readonly(self.st) != 0 }
        }
    }

    #[derive(Debug, Copy, Clone)]
    struct SqliteBlob {
        blob: *mut sqlite3_blob,
    }

    impl SqliteBlob {
        fn bytes(self) -> c_int {
            unsafe { sqlite3_blob_bytes(self.blob) }
        }

        fn write(self, buf: *const c_void, length: c_int, offset: c_int) -> c_int {
            unsafe { sqlite3_blob_write(self.blob, buf, length, offset) }
        }

        fn read(self, buf: *mut c_void, length: c_int, offset: c_int) -> c_int {
            unsafe { sqlite3_blob_read(self.blob, buf, length, offset) }
        }

        fn read_single(self, byte: &mut u8, offset: c_int) -> c_int {
            self.read(byte as *mut u8 as *mut _, 1, offset)
        }

        fn write_single(self, byte: u8, offset: c_int) -> c_int {
            self.write(&byte as *const u8 as *const _, 1, offset)
        }
    }

    #[derive(Copy, Clone)]
    struct SqliteContext {
        ctx: *mut sqlite3_context,
    }

    impl From<*mut sqlite3_context> for SqliteContext {
        fn from(ctx: *mut sqlite3_context) -> Self {
            Self { ctx }
        }
    }

    impl SqliteContext {
        fn user_data<T>(self) -> *mut T {
            unsafe { sqlite3_user_data(self.ctx).cast() }
        }

        fn aggregate_context<T>(self) -> *mut T {
            unsafe { sqlite3_aggregate_context(self.ctx, std::mem::size_of::<T>() as c_int).cast() }
        }

        fn result_exception(self, vm: &VirtualMachine, exc: PyBaseExceptionRef, msg: &str) {
            if exc.fast_isinstance(vm.ctx.exceptions.memory_error) {
                unsafe { sqlite3_result_error_nomem(self.ctx) }
            } else if exc.fast_isinstance(vm.ctx.exceptions.overflow_error) {
                unsafe { sqlite3_result_error_toobig(self.ctx) }
            } else {
                unsafe { sqlite3_result_error(self.ctx, msg.as_ptr().cast(), -1) }
            }
            if enable_traceback().load(Ordering::Relaxed) {
                vm.print_exception(exc);
            }
        }

        fn db_handle(self) -> *mut sqlite3 {
            unsafe { sqlite3_context_db_handle(self.ctx) }
        }

        fn result_from_object(self, val: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            unsafe {
                if vm.is_none(val) {
                    sqlite3_result_null(self.ctx)
                } else if let Some(val) = val.payload::<PyInt>() {
                    sqlite3_result_int64(self.ctx, val.try_to_primitive(vm)?)
                } else if let Some(val) = val.payload::<PyFloat>() {
                    sqlite3_result_double(self.ctx, val.to_f64())
                } else if let Some(val) = val.payload::<PyStr>() {
                    let (ptr, len) = str_to_ptr_len(val, vm)?;
                    sqlite3_result_text(self.ctx, ptr, len, SQLITE_TRANSIENT())
                } else if let Ok(buffer) = PyBuffer::try_from_borrowed_object(vm, val) {
                    let (ptr, len) = buffer_to_ptr_len(&buffer, vm)?;
                    sqlite3_result_blob(self.ctx, ptr, len, SQLITE_TRANSIENT())
                } else {
                    return Err(new_programming_error(
                        vm,
                        "result type not support".to_owned(),
                    ));
                }
            }
            Ok(())
        }
    }

    fn value_to_object(val: *mut sqlite3_value, db: *mut sqlite3, vm: &VirtualMachine) -> PyResult {
        let obj = unsafe {
            match sqlite3_value_type(val) {
                SQLITE_INTEGER => vm.ctx.new_int(sqlite3_value_int64(val)).into(),
                SQLITE_FLOAT => vm.ctx.new_float(sqlite3_value_double(val)).into(),
                SQLITE_TEXT => {
                    let text =
                        ptr_to_vec(sqlite3_value_text(val), sqlite3_value_bytes(val), db, vm)?;
                    let text = String::from_utf8(text).map_err(|_| {
                        vm.new_value_error("invalid utf-8 with SQLITE_TEXT".to_owned())
                    })?;
                    vm.ctx.new_str(text).into()
                }
                SQLITE_BLOB => {
                    let blob = ptr_to_vec(
                        sqlite3_value_blob(val).cast(),
                        sqlite3_value_bytes(val),
                        db,
                        vm,
                    )?;
                    vm.ctx.new_bytes(blob).into()
                }
                _ => vm.ctx.none(),
            }
        };
        Ok(obj)
    }

    fn ptr_to_str<'a>(p: *const libc::c_char, vm: &VirtualMachine) -> PyResult<&'a str> {
        if p.is_null() {
            return Err(vm.new_memory_error("string pointer is null".to_owned()));
        }
        unsafe { CStr::from_ptr(p).to_str() }
            .map_err(|_| vm.new_value_error("Invalid UIF-8 codepoint".to_owned()))
    }

    fn ptr_to_string(
        p: *const u8,
        nbytes: c_int,
        db: *mut sqlite3,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        let s = ptr_to_vec(p, nbytes, db, vm)?;
        String::from_utf8(s).map_err(|_| vm.new_value_error("invalid utf-8".to_owned()))
    }

    fn ptr_to_vec(
        p: *const u8,
        nbytes: c_int,
        db: *mut sqlite3,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        if p.is_null() {
            if !db.is_null() && unsafe { sqlite3_errcode(db) } == SQLITE_NOMEM {
                Err(vm.new_memory_error("sqlite out of memory".to_owned()))
            } else {
                Ok(vec![])
            }
        } else if nbytes < 0 {
            Err(vm.new_system_error("negative size with ptr".to_owned()))
        } else {
            Ok(unsafe { std::slice::from_raw_parts(p.cast(), nbytes as usize) }.to_vec())
        }
    }

    fn str_to_ptr_len(s: &PyStr, vm: &VirtualMachine) -> PyResult<(*const libc::c_char, i32)> {
        let len = c_int::try_from(s.byte_len())
            .map_err(|_| vm.new_overflow_error("TEXT longer than INT_MAX bytes".to_owned()))?;
        let ptr = s.as_str().as_ptr().cast();
        Ok((ptr, len))
    }

    fn buffer_to_ptr_len(buffer: &PyBuffer, vm: &VirtualMachine) -> PyResult<(*const c_void, i32)> {
        let bytes = buffer.as_contiguous().ok_or_else(|| {
            vm.new_buffer_error("underlying buffer is not C-contiguous".to_owned())
        })?;
        let len = c_int::try_from(bytes.len())
            .map_err(|_| vm.new_overflow_error("BLOB longer than INT_MAX bytes".to_owned()))?;
        let ptr = bytes.as_ptr().cast();
        Ok((ptr, len))
    }

    fn exception_type_from_errcode(errcode: c_int, vm: &VirtualMachine) -> &'static Py<PyType> {
        match errcode {
            SQLITE_INTERNAL | SQLITE_NOTFOUND => internal_error_type(),
            SQLITE_NOMEM => vm.ctx.exceptions.memory_error,
            SQLITE_ERROR | SQLITE_PERM | SQLITE_ABORT | SQLITE_BUSY | SQLITE_LOCKED
            | SQLITE_READONLY | SQLITE_INTERRUPT | SQLITE_IOERR | SQLITE_FULL | SQLITE_CANTOPEN
            | SQLITE_PROTOCOL | SQLITE_EMPTY | SQLITE_SCHEMA => operational_error_type(),
            SQLITE_CORRUPT => database_error_type(),
            SQLITE_TOOBIG => data_error_type(),
            SQLITE_CONSTRAINT | SQLITE_MISMATCH => integrity_error_type(),
            SQLITE_MISUSE | SQLITE_RANGE => interface_error_type(),
            _ => database_error_type(),
        }
    }

    fn name_from_errcode(errcode: c_int) -> &'static str {
        for (name, code) in ERROR_CODES {
            if *code == errcode {
                return name;
            }
        }
        "unknown error code"
    }

    fn raise_exception(
        typ: PyTypeRef,
        errcode: c_int,
        msg: String,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        let dict = vm.ctx.new_dict();
        if let Err(e) = dict.set_item("sqlite_errorcode", vm.ctx.new_int(errcode).into(), vm) {
            return e;
        }
        let errname = name_from_errcode(errcode);
        if let Err(e) = dict.set_item("sqlite_errorname", vm.ctx.new_str(errname).into(), vm) {
            return e;
        }

        vm.new_exception_msg_dict(typ, msg, dict)
    }

    static BEGIN_STATEMENTS: &[&[u8]] = &[
        b"BEGIN ",
        b"BEGIN DEFERRED",
        b"BEGIN IMMEDIATE",
        b"BEGIN EXCLUSIVE",
    ];

    fn begin_statement_ptr_from_isolation_level(
        s: &PyStr,
        vm: &VirtualMachine,
    ) -> PyResult<*const libc::c_char> {
        BEGIN_STATEMENTS
            .iter()
            .find(|&&x| x[6..].eq_ignore_ascii_case(s.as_str().as_bytes()))
            .map(|&x| x.as_ptr().cast())
            .ok_or_else(|| {
                vm.new_value_error(
                    "isolation_level string must be '', 'DEFERRED', 'IMMEDIATE', or 'EXCLUSIVE'"
                        .to_owned(),
                )
            })
    }

    fn lstrip_sql(sql: &[u8]) -> Option<&[u8]> {
        let mut pos = sql;
        loop {
            match pos.first()? {
                b' ' | b'\t' | b'\x0c' | b'\n' | b'\r' => {
                    pos = &pos[1..];
                }
                b'-' => {
                    if *pos.get(1)? == b'-' {
                        // line comments
                        pos = &pos[2..];
                        while *pos.first()? != b'\n' {
                            pos = &pos[1..];
                        }
                    } else {
                        return Some(pos);
                    }
                }
                b'/' => {
                    if *pos.get(1)? == b'*' {
                        // c style comments
                        pos = &pos[2..];
                        while *pos.first()? != b'*' || *pos.get(1)? != b'/' {
                            pos = &pos[1..];
                        }
                    } else {
                        return Some(pos);
                    }
                }
                _ => return Some(pos),
            }
        }
    }
}
