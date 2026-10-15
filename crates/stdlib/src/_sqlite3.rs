// spell-checker:ignore libsqlite3 threadsafety PYSQLITE decltypes colnames collseq cantinit dirtywal
// spell-checker:ignore corruptfs narg setinputsizes setoutputsize lastrowid arraysize executemany
// spell-checker:ignore blobopen executescript iterdump getlimit setlimit errorcode errorname
// spell-checker:ignore rowid rowcount fetchone fetchmany fetchall errcode errname vtable pagecount
// spell-checker:ignore autocommit libversion toobig errmsg nomem threadsafe longlong vdbe reindex
// spell-checker:ignore savepoint cantopen ioerr nolfs nomem notadb notfound fullpath notempdir vtab
// spell-checker:ignore checkreservedlock noent fstat rdlock shmlock shmmap shmopen shmsize sharedcache
// spell-checker:ignore cantlock commithook foreignkey notnull primarykey gettemppath autoindex convpath
// spell-checker:ignore dbmoved vnode nbytes

pub(crate) use _sqlite3::module_def;

#[pymodule]
mod _sqlite3 {
    use core::{
        ffi::{CStr, c_int, c_longlong, c_uint, c_void},
        fmt::Debug,
        ops::{Deref, DerefMut},
        ptr::{NonNull, null, null_mut},
    };
    use libsqlite3_sys::{
        SQLITE_BLOB, SQLITE_DETERMINISTIC, SQLITE_FLOAT, SQLITE_INTEGER, SQLITE_NULL,
        SQLITE_OPEN_CREATE, SQLITE_OPEN_FULLMUTEX, SQLITE_OPEN_READWRITE, SQLITE_OPEN_URI,
        SQLITE_TEXT, SQLITE_TRACE_STMT, SQLITE_TRANSIENT, SQLITE_UTF8, sqlite3,
        sqlite3_aggregate_context, sqlite3_backup_finish, sqlite3_backup_init,
        sqlite3_backup_pagecount, sqlite3_backup_remaining, sqlite3_backup_step, sqlite3_bind_blob,
        sqlite3_bind_double, sqlite3_bind_int64, sqlite3_bind_null, sqlite3_bind_parameter_count,
        sqlite3_bind_parameter_name, sqlite3_bind_text, sqlite3_blob, sqlite3_blob_bytes,
        sqlite3_blob_close, sqlite3_blob_open, sqlite3_blob_read, sqlite3_blob_write,
        sqlite3_busy_timeout, sqlite3_changes, sqlite3_clear_bindings, sqlite3_column_blob,
        sqlite3_column_bytes, sqlite3_column_count, sqlite3_column_decltype, sqlite3_column_double,
        sqlite3_column_int64, sqlite3_column_name, sqlite3_column_text, sqlite3_column_type,
        sqlite3_complete, sqlite3_context, sqlite3_context_db_handle, sqlite3_create_collation_v2,
        sqlite3_create_function_v2, sqlite3_create_window_function, sqlite3_data_count,
        sqlite3_db_config, sqlite3_db_handle, sqlite3_db_mutex, sqlite3_errcode, sqlite3_errmsg,
        sqlite3_exec, sqlite3_expanded_sql, sqlite3_extended_errcode, sqlite3_finalize,
        sqlite3_free, sqlite3_get_autocommit, sqlite3_interrupt, sqlite3_last_insert_rowid,
        sqlite3_libversion, sqlite3_limit, sqlite3_mutex_enter, sqlite3_mutex_leave,
        sqlite3_mutex_try, sqlite3_open_v2, sqlite3_prepare_v2, sqlite3_progress_handler,
        sqlite3_reset, sqlite3_result_blob, sqlite3_result_double, sqlite3_result_error,
        sqlite3_result_error_nomem, sqlite3_result_error_toobig, sqlite3_result_int64,
        sqlite3_result_null, sqlite3_result_text, sqlite3_set_authorizer, sqlite3_sleep,
        sqlite3_step, sqlite3_stmt, sqlite3_stmt_busy, sqlite3_stmt_readonly, sqlite3_threadsafe,
        sqlite3_total_changes, sqlite3_trace_v2, sqlite3_user_data, sqlite3_value,
        sqlite3_value_blob, sqlite3_value_bytes, sqlite3_value_double, sqlite3_value_int64,
        sqlite3_value_text, sqlite3_value_type,
    };
    use malachite_bigint::Sign;
    use num_traits::ToPrimitive;
    use rustpython_common::{
        atomic::{Ordering, PyAtomic, Radium},
        hash::PyHash,
        lock::{PyMappedMutexGuard, PyMutex, PyMutexGuard},
        rc::PyRc,
        refcount::{try_defer_drop, with_deferred_drops},
        static_cell,
    };
    use rustpython_vm::{
        __exports::paste,
        AsObject, Py, PyAtomicRef, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
        TryFromBorrowedObject, TryFromObject, VirtualMachine, atomic_func,
        builtins::{
            PyBaseException, PyBaseExceptionRef, PyByteArray, PyBytes, PyDict, PyDictRef, PyFloat,
            PyInt, PyIntRef, PyModule, PySlice, PyStr, PyStrRef, PyTuple, PyTupleRef, PyType,
            PyTypeRef, PyUtf8Str, PyUtf8StrRef,
        },
        function::{
            ArgCallable, ArgIterable, FsPath, FuncArgs, KwArgs, OptionalArg, PyComparisonValue,
            PySetterValue, TimeoutSeconds,
        },
        object::{Traverse, TraverseFn},
        protocol::{
            PyBuffer, PyIterReturn, PyMappingMethods, PyNumberMethods, PySequence,
            PySequenceMethods,
        },
        sliceable::{SaturatedSliceIter, SliceableSequenceOp},
        types::{
            AsMapping, AsNumber, AsSequence, Callable, Comparable, Constructor, Hashable,
            Initializer, IterNext, Iterable, PyComparisonOp, SelfIter,
        },
        utils::ToCString,
    };
    use std::thread::ThreadId;

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
                        vm.new_exception_msg([<$x:snake _type>]().to_owned(), msg.into())
                    }
                    fn [<$x:snake _type>]() -> &'static Py<PyType> {
                        [<$x:snake:upper>].get().expect("exception type not initialize")
                    }
                )*
                fn setup_module_exceptions(module: &PyObject, vm: &VirtualMachine) {
                    $(
                        #[allow(clippy::redundant_closure_call)]
                        let exception = [<$x:snake:upper>].get_or_init(|| {
                            let base = $base(vm);
                            vm.ctx.new_exception_type("_sqlite3", stringify!($x), Some(vec![base.to_owned()]))
                        });
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
    const LEGACY_TRANSACTION_CONTROL: c_int = -1;

    #[pyattr]
    use libsqlite3_sys::{
        SQLITE_ALTER_TABLE, SQLITE_ANALYZE, SQLITE_ATTACH, SQLITE_CREATE_INDEX,
        SQLITE_CREATE_TABLE, SQLITE_CREATE_TEMP_INDEX, SQLITE_CREATE_TEMP_TABLE,
        SQLITE_CREATE_TEMP_TRIGGER, SQLITE_CREATE_TEMP_VIEW, SQLITE_CREATE_TRIGGER,
        SQLITE_CREATE_VIEW, SQLITE_CREATE_VTABLE, SQLITE_DBCONFIG_DEFENSIVE,
        SQLITE_DBCONFIG_DQS_DDL, SQLITE_DBCONFIG_DQS_DML, SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE,
        SQLITE_DBCONFIG_ENABLE_ATTACH_WRITE, SQLITE_DBCONFIG_ENABLE_COMMENTS,
        SQLITE_DBCONFIG_ENABLE_FKEY, SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER,
        SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION, SQLITE_DBCONFIG_ENABLE_QPSG,
        SQLITE_DBCONFIG_ENABLE_TRIGGER, SQLITE_DBCONFIG_ENABLE_VIEW,
        SQLITE_DBCONFIG_LEGACY_ALTER_TABLE, SQLITE_DBCONFIG_LEGACY_FILE_FORMAT,
        SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, SQLITE_DBCONFIG_RESET_DATABASE,
        SQLITE_DBCONFIG_REVERSE_SCANORDER, SQLITE_DBCONFIG_STMT_SCANSTATUS,
        SQLITE_DBCONFIG_TRIGGER_EQP, SQLITE_DBCONFIG_TRUSTED_SCHEMA,
        SQLITE_DBCONFIG_WRITABLE_SCHEMA, SQLITE_DELETE, SQLITE_DENY, SQLITE_DETACH,
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

    /// Autocommit mode setting for sqlite3 connections.
    /// - Legacy (default): use isolation_level to control transactions
    /// - Enabled: autocommit mode (no automatic transactions)
    /// - Disabled: manual commit mode
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    enum AutocommitMode {
        #[default]
        Legacy,
        Enabled,
        Disabled,
    }

    impl TryFromBorrowedObject<'_> for AutocommitMode {
        fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Self> {
            if obj.is(&vm.ctx.true_value) {
                Ok(Self::Enabled)
            } else if obj.is(&vm.ctx.false_value) {
                Ok(Self::Disabled)
            } else if let Ok(val) = obj.try_to_value::<c_int>(vm) {
                if val == LEGACY_TRANSACTION_CONTROL {
                    Ok(Self::Legacy)
                } else {
                    Err(vm.new_value_error(format!(
                        "autocommit must be True, False, or sqlite3.LEGACY_TRANSACTION_CONTROL, not {val}"
                    )))
                }
            } else {
                Err(vm.new_value_error(format!(
                    "autocommit must be True, False, or sqlite3.LEGACY_TRANSACTION_CONTROL, not {}",
                    obj.class().name()
                )))
            }
        }
    }

    struct IsolationLevelArg(Option<PyStrRef>);

    impl TryFromObject for IsolationLevelArg {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            if vm.is_none(&obj) {
                return Ok(Self(None));
            }
            obj.downcast::<PyStr>()
                .map_err(|_| vm.new_type_error("isolation_level must be str or None".to_owned()))
                .and_then(|s| {
                    if s.contains_nuls() {
                        Err(vm.new_value_error("isolation_level contains interior NULs".to_owned()))
                    } else {
                        Ok(Self(Some(s)))
                    }
                })
        }
    }

    #[derive(FromArgs)]
    struct ConnectArgs {
        #[pyarg(any)]
        database: FsPath,
        #[pyarg(any, default = TimeoutSeconds::new(5.0))]
        timeout: TimeoutSeconds,
        #[pyarg(any, default = 0)]
        detect_types: c_int,
        #[pyarg(any, default = "")]
        isolation_level: IsolationLevelArg,
        #[pyarg(any, default = true)]
        check_same_thread: bool,
        #[pyarg(any, default = Connection::class(&vm.ctx).to_owned(), py_default = "ConnectionType")]
        factory: PyTypeRef,
        #[pyarg(any, default = 128)]
        cached_statements: c_int,
        #[pyarg(any, default = false)]
        uri: bool,
        #[pyarg(any, default)]
        autocommit: AutocommitMode,
    }

    unsafe impl Traverse for ConnectArgs {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            self.isolation_level.0.traverse(tracer_fn);
            self.factory.traverse(tracer_fn);
        }
    }

    #[derive(FromArgs)]
    struct BackupArgs {
        #[pyarg(any)]
        target: PyRef<Connection>,
        #[pyarg(named, default = -1)]
        pages: c_int,
        #[pyarg(named, optional)]
        progress: Option<PyObjectRef>,
        #[pyarg(named, optional)]
        name: Option<PyStrRef>,
        #[pyarg(named, default = 0.250)]
        sleep: f64,
    }

    unsafe impl Traverse for BackupArgs {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            self.progress.traverse(tracer_fn);
            self.name.traverse(tracer_fn);
        }
    }

    #[derive(FromArgs)]
    struct IterDumpArgs {
        #[pyarg(named, optional)]
        filter: Option<PyStrRef>,
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
        rowid: i64,
        #[pyarg(named, default)]
        readonly: bool,
        #[pyarg(named, default = "main")]
        name: PyStrRef,
    }

    #[derive(FromArgs)]
    struct CursorArgs {
        #[pyarg(any, default)]
        factory: OptionalArg<PyObjectRef>,
    }

    struct CallbackData {
        obj: NonNull<PyObject>,
    }

    impl CallbackData {
        fn new(obj: PyObjectRef, vm: &VirtualMachine) -> Option<Self> {
            (!vm.is_none(&obj)).then_some(Self {
                obj: obj.into_raw(),
            })
        }

        fn with_vm<R>(&self, f: impl Fn(&PyObject, &VirtualMachine) -> R) -> Option<R> {
            let obj = unsafe { self.obj.as_ref() };
            // A shared connection may invoke this callback on a different thread
            // after the registering thread's VM has already been destroyed.
            rustpython_vm::vm::thread::with_vm(obj, |vm| f(obj, vm))
        }

        // The SQLite mutex keeps the registration alive until this reference is acquired.
        unsafe fn registered(data: *mut c_void) -> PyRc<Self> {
            let data = data.cast::<Self>();
            unsafe {
                PyRc::increment_strong_count(data);
                PyRc::from_raw(data)
            }
        }

        fn with_context(&self, context: SqliteContext, f: impl Fn(&PyObject, &VirtualMachine)) {
            if self.with_vm(f).is_none() {
                unsafe {
                    sqlite3_result_error(
                        context.ctx,
                        c"SQLite callback has no active interpreter".as_ptr(),
                        -1,
                    )
                };
            }
        }

        unsafe extern "C" fn destructor(data: *mut c_void) {
            drop(unsafe { Box::from_raw(data.cast::<Self>()) });
        }

        unsafe extern "C" fn func_callback(
            context: *mut sqlite3_context,
            argc: c_int,
            argv: *mut *mut sqlite3_value,
        ) {
            let context = SqliteContext::from(context);
            let data = unsafe { &*context.user_data::<Self>() };
            data.with_context(context, |func, vm| {
                let args = unsafe { core::slice::from_raw_parts(argv, argc as usize) };

                let f = || -> PyResult<()> {
                    let db = context.db_handle();
                    let args = args
                        .iter()
                        .copied()
                        .map(|val| value_to_object(val, db, vm))
                        .collect::<PyResult<Vec<PyObjectRef>>>()?;

                    let val = func.call(args, vm)?;

                    context.result_from_object(&val, vm)
                };

                if let Err(exc) = f() {
                    context.result_exception(vm, exc, c"user-defined function raised exception")
                }
            });
        }

        unsafe extern "C" fn step_callback(
            context: *mut sqlite3_context,
            argc: c_int,
            argv: *mut *mut sqlite3_value,
        ) {
            let context = SqliteContext::from(context);
            let data = unsafe { &*context.user_data::<Self>() };
            data.with_context(context, |cls, vm| {
                let args = unsafe { core::slice::from_raw_parts(argv, argc as usize) };
                let instance = context.aggregate_context(true);
                if instance.is_null() {
                    unsafe { sqlite3_result_error_nomem(context.ctx) };
                    return;
                }
                if unsafe { (*instance).is_null() } {
                    match cls.call((), vm) {
                        Ok(obj) => unsafe { *instance = obj.into_raw().as_ptr() },
                        Err(exc) => {
                            return context.result_exception(
                                vm,
                                exc,
                                c"user-defined aggregate's '__init__' method raised error",
                            );
                        }
                    }
                }
                let instance = unsafe { &**instance };

                Self::call_method_with_args(context, instance, "step", args, vm);
            });
        }

        unsafe extern "C" fn finalize_callback(context: *mut sqlite3_context) {
            let context = SqliteContext::from(context);
            let data = unsafe { &*context.user_data::<Self>() };
            let instance = context.aggregate_context(false);
            if instance.is_null() {
                return;
            }
            // xFinal releases the reference acquired by xStep, even if the
            // query or finalize() fails. Clear the slot before calling Python.
            let object = unsafe { instance.replace(core::ptr::null()) };
            let Some(object) = NonNull::new(object.cast_mut()) else {
                return;
            };
            let instance = unsafe { PyObjectRef::from_raw(object) };

            data.with_context(context, |_, vm| {
                Self::callback_result_from_method(context, &instance, "finalize", vm);
            });
            // __del__ and weakref callbacks must run after the caller releases
            // its cursor, statement and connection locks.
            try_defer_drop(move || drop(instance));
        }

        unsafe extern "C" fn collation_callback(
            data: *mut c_void,
            a_len: c_int,
            a_ptr: *const c_void,
            b_len: c_int,
            b_ptr: *const c_void,
        ) -> c_int {
            let data = unsafe { &*data.cast::<Self>() };
            data.with_vm(|callable, vm| {
                let f = || -> PyResult<c_int> {
                    let text1 = ptr_to_string(a_ptr.cast(), a_len, null_mut(), vm)?;
                    let text1 = vm.ctx.new_str(text1);
                    let text2 = ptr_to_string(b_ptr.cast(), b_len, null_mut(), vm)?;
                    let text2 = vm.ctx.new_str(text2);

                    let val = callable.call((text1, text2), vm)?;
                    let Some(val) = val.number().index(vm) else {
                        return Ok(0);
                    };

                    let val = match val?.as_bigint().sign() {
                        Sign::Plus => 1,
                        Sign::Minus => -1,
                        Sign::NoSign => 0,
                    };

                    Ok(val)
                };

                f().unwrap_or(0)
            })
            .unwrap_or(0)
        }

        unsafe extern "C" fn value_callback(context: *mut sqlite3_context) {
            let context = SqliteContext::from(context);
            let data = unsafe { &*context.user_data::<Self>() };
            data.with_context(context, |_, vm| {
                let instance = context.aggregate_context(false);
                let Some(instance) = (unsafe { instance.as_ref().and_then(|obj| obj.as_ref()) })
                else {
                    return;
                };

                Self::callback_result_from_method(context, instance, "value", vm);
            });
        }

        unsafe extern "C" fn inverse_callback(
            context: *mut sqlite3_context,
            argc: c_int,
            argv: *mut *mut sqlite3_value,
        ) {
            let context = SqliteContext::from(context);
            let data = unsafe { &*context.user_data::<Self>() };
            data.with_context(context, |_, vm| {
                let args = unsafe { core::slice::from_raw_parts(argv, argc as usize) };
                let instance = context.aggregate_context(false);
                let Some(instance) = (unsafe { instance.as_ref().and_then(|obj| obj.as_ref()) })
                else {
                    return;
                };

                Self::call_method_with_args(context, instance, "inverse", args, vm);
            });
        }

        unsafe extern "C" fn authorizer_callback(
            data: *mut c_void,
            action: c_int,
            arg1: *const libc::c_char,
            arg2: *const libc::c_char,
            db_name: *const libc::c_char,
            access: *const libc::c_char,
        ) -> c_int {
            let data = unsafe { Self::registered(data) };
            data.with_vm(|callable, vm| {
                let f = || -> PyResult<c_int> {
                    let arg1 = ptr_to_str_or_none(arg1, vm)?;
                    let arg2 = ptr_to_str_or_none(arg2, vm)?;
                    let db_name = ptr_to_str_or_none(db_name, vm)?;
                    let access = ptr_to_str_or_none(access, vm)?;

                    let val = callable.call((action, arg1, arg2, db_name, access), vm)?;
                    let Some(val) = val.downcast_ref::<PyInt>() else {
                        return Ok(SQLITE_DENY);
                    };
                    val.try_to_primitive::<c_int>(vm)
                };

                f().unwrap_or(SQLITE_DENY)
            })
            .unwrap_or(SQLITE_DENY)
        }

        unsafe extern "C" fn trace_callback(
            _typ: c_uint,
            data: *mut c_void,
            stmt: *mut c_void,
            sql: *mut c_void,
        ) -> c_int {
            let data = unsafe { Self::registered(data) };
            data.with_vm(|callable, vm| {
                let f = || -> PyResult<()> {
                    let stmt = {
                        let expanded = scopeguard::guard(
                            unsafe { sqlite3_expanded_sql(stmt.cast()) },
                            |sql| unsafe { sqlite3_free(sql.cast()) },
                        );
                        let text =
                            ptr_to_str(*expanded, vm).or_else(|_| ptr_to_str(sql.cast(), vm))?;
                        vm.ctx.new_str(text)
                    };
                    callable.call((stmt,), vm)?;
                    Ok(())
                };
                let _ = f();
                0
            })
            .unwrap_or(0)
        }

        unsafe extern "C" fn progress_callback(data: *mut c_void) -> c_int {
            let data = unsafe { Self::registered(data) };
            data.with_vm(|callable, vm| {
                if let Ok(val) = callable.call((), vm)
                    && let Ok(val) = val.is_true(vm)
                {
                    return val as c_int;
                }
                -1
            })
            .unwrap_or(-1)
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
                        CStr::from_bytes_until_nul(
                            format!("user-defined aggregate's '{name}' method not defined\0")
                                .as_bytes(),
                        )
                        .unwrap(),
                    )
                } else {
                    context.result_exception(
                        vm,
                        exc,
                        CStr::from_bytes_until_nul(
                            format!("user-defined aggregate's '{name}' method raised error\0")
                                .as_bytes(),
                        )
                        .unwrap(),
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
                    .copied()
                    .map(|val| value_to_object(val, db, vm))
                    .collect::<PyResult<Vec<PyObjectRef>>>()?;
                vm.call_method(instance, name, args).map(drop)
            };

            if let Err(exc) = f() {
                if exc.fast_isinstance(vm.ctx.exceptions.attribute_error) {
                    context.result_exception(
                        vm,
                        exc,
                        CStr::from_bytes_until_nul(
                            format!("user-defined aggregate's '{name}' method not defined\0")
                                .as_bytes(),
                        )
                        .unwrap(),
                    )
                } else {
                    context.result_exception(
                        vm,
                        exc,
                        CStr::from_bytes_until_nul(
                            format!("user-defined aggregate's '{name}' method raised error\0")
                                .as_bytes(),
                        )
                        .unwrap(),
                    )
                }
            }
        }
    }

    impl Drop for CallbackData {
        fn drop(&mut self) {
            let obj = unsafe { PyObjectRef::from_raw(self.obj) };
            // The last active callback may outlive its registration. Release the
            // Python reference after the outer operation's native/Rust locks.
            try_defer_drop(move || drop(obj));
        }
    }

    #[pyfunction]
    fn connect(args: ConnectArgs, vm: &VirtualMachine) -> PyResult {
        let factory = args.factory.clone();
        let conn = Connection::py_new(&factory, args, vm)?;
        conn.into_ref_with_type(vm, factory).map(Into::into)
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
        if typ.is(PyInt::class(&vm.ctx))
            || typ.is(PyFloat::class(&vm.ctx))
            || typ.is(PyStr::class(&vm.ctx))
            || typ.is(PyByteArray::class(&vm.ctx))
        {
            let _ = BASE_TYPE_ADAPTED.set(());
        }
        let protocol = PrepareProtocol::class(&vm.ctx).to_owned();
        let key = vm.ctx.new_tuple(vec![typ.into(), protocol.into()]);
        adapters().set_item(key.as_object(), adapter.into(), vm)
    }

    #[pyfunction]
    fn register_converter(
        typename: PyStrRef,
        converter: ArgCallable,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let name = typename.expect_str().to_uppercase();
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
            return adapter.call((obj,), vm);
        }
        if let Ok(adapter) = proto.get_attr("__adapt__", vm) {
            match adapter.call((obj,), vm) {
                Ok(val) => {
                    if !vm.is_none(&val) {
                        return Ok(val);
                    }
                }
                Err(exc) => {
                    if !exc.fast_isinstance(vm.ctx.exceptions.type_error) {
                        return Err(exc);
                    }
                }
            }
        }
        if let Ok(adapter) = obj.get_attr("__conform__", vm) {
            match adapter.call((proto,), vm) {
                Ok(val) => {
                    if !vm.is_none(&val) {
                        return Ok(val);
                    }
                }
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
        if matches!(proto, OptionalArg::Present(None)) {
            return if let OptionalArg::Present(alt) = alt {
                Ok(alt)
            } else {
                Err(new_programming_error(vm, "can't adapt".to_owned()))
            };
        }

        let proto = proto
            .flatten()
            .unwrap_or_else(|| PrepareProtocol::class(&vm.ctx).to_owned());

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

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);

        for (name, code) in ERROR_CODES {
            let name = vm.ctx.intern_str(*name);
            let code = vm.new_pyobj(*code);
            module.set_attr(name, code, vm)?;
        }

        setup_module_exceptions(module.as_object(), vm);

        let _ = CONVERTERS.set(vm.ctx.new_dict());
        let _ = ADAPTERS.set(vm.ctx.new_dict());
        let _ = USER_FUNCTION_EXCEPTION.set(PyAtomicRef::from(None));
        let _ = ENABLE_TRACEBACK.set(Radium::new(false));

        module.set_attr("converters", converters().to_owned(), vm)?;
        module.set_attr("adapters", adapters().to_owned(), vm)?;

        Ok(())
    }

    #[pyattr]
    #[pyclass(name)]
    #[derive(PyPayload)]
    struct Connection {
        db: PyMutex<Option<PyRc<Sqlite>>>,
        statement_cache: PyMutex<StatementCache>,
        initialized: PyAtomic<bool>,
        detect_types: PyAtomic<c_int>,
        isolation_level: PyAtomicRef<Option<PyStr>>,
        check_same_thread: PyAtomic<bool>,
        thread_ident: PyMutex<ThreadId>, // TODO: Use atomic
        row_factory: PyAtomicRef<Option<PyObject>>,
        text_factory: PyAtomicRef<PyObject>,
        autocommit: PyMutex<AutocommitMode>,
    }

    impl Debug for Connection {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> Result<(), core::fmt::Error> {
            write!(f, "Sqlite3 Connection")
        }
    }

    impl Constructor for Connection {
        type Args = ConnectArgs;

        fn py_new(cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let text_factory = PyStr::class(&vm.ctx).to_owned().into_object();

            // For non-subclassed Connection, initialize in __new__
            // For subclassed Connection, leave db as None and require __init__ to be called
            let is_base_class = cls.is(Self::class(&vm.ctx));

            let db = if is_base_class {
                // Initialize immediately for base class
                Some(Self::initialize_db(&args, vm)?)
            } else {
                // For subclasses, require __init__ to be called
                None
            };

            let initialized = db.is_some();

            Ok(Self {
                db: PyMutex::new(db),
                statement_cache: PyMutex::new(StatementCache::new(args.cached_statements)),
                initialized: Radium::new(initialized),
                detect_types: Radium::new(args.detect_types),
                isolation_level: PyAtomicRef::from(args.isolation_level.0),
                check_same_thread: Radium::new(args.check_same_thread),
                thread_ident: PyMutex::new(std::thread::current().id()),
                row_factory: PyAtomicRef::from(None),
                text_factory: PyAtomicRef::from(text_factory),
                autocommit: PyMutex::new(args.autocommit),
            })
        }
    }

    impl Callable for Connection {
        type Args = FuncArgs;

        fn call(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            with_deferred_drops(|| Self::call_inner(zelf, args, vm))
        }
    }

    impl Connection {
        fn call_inner(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let _ = zelf.db_lock(vm)?;

            let (sql,): (PyUtf8StrRef,) = args.bind(vm)?;

            if let Some(stmt) = Statement::new(zelf, sql, vm)? {
                Ok(stmt.into_ref(&vm.ctx).into())
            } else {
                Ok(vm.ctx.none())
            }
        }
    }

    impl Initializer for Connection {
        type Args = ConnectArgs;

        fn init(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            loop {
                with_deferred_drops(|| {
                    Radium::store(&zelf.initialized, false, Ordering::Release);

                    // Reset factories to their defaults, matching CPython's behavior.
                    zelf.reset_factories(vm);
                    zelf.drop_db(vm)
                })?;
                // A finalizer may itself reinitialize the connection. Dispose of
                // that database too before publishing the requested replacement.
                if zelf.db.lock().is_none() {
                    break;
                }
            }

            // Dispose of the old callbacks before publishing the replacement.
            // Nested native calls still defer their drops to the outer operation.
            // Attempt to open the new database before mutating other state so failures leave
            // the connection uninitialized (and subsequent operations raise ProgrammingError).
            let db = Self::initialize_db(&args, vm)?;

            let ConnectArgs {
                detect_types,
                isolation_level,
                check_same_thread,
                autocommit,
                cached_statements,
                ..
            } = args;

            let default_text_factory = PyStr::class(&vm.ctx).to_owned().into_object();
            loop {
                let mut current = zelf.db.lock();
                if current.is_none() {
                    let previous = unsafe { zelf.isolation_level.swap(isolation_level.0.clone()) };
                    let row_factory = unsafe { zelf.row_factory.swap(None) };
                    let text_factory =
                        unsafe { zelf.text_factory.swap(default_text_factory.clone()) };
                    if previous.as_ref().is_some_and(|previous| {
                        !isolation_level
                            .0
                            .as_ref()
                            .is_some_and(|requested| previous.is(requested))
                    }) || row_factory.is_some()
                        || !text_factory.is(&default_text_factory)
                    {
                        // Factories and str subclasses can finalize too. Retire
                        // them before publication, then recheck any reentrant changes.
                        drop(current);
                        with_deferred_drops(|| drop((previous, row_factory, text_factory)));
                        continue;
                    }

                    // No destructor-capable owner remains to release here.
                    // Publish the settings and database under the same lock.
                    zelf.statement_cache.lock().capacity = cached_statements.max(0) as usize;
                    zelf.detect_types.store(detect_types, Ordering::Relaxed);
                    zelf.check_same_thread
                        .store(check_same_thread, Ordering::Relaxed);
                    *zelf.autocommit.lock() = autocommit;
                    *zelf.thread_ident.lock() = std::thread::current().id();
                    *current = Some(db);
                    Radium::store(&zelf.initialized, true, Ordering::Release);
                    return Ok(());
                }
                drop(current);

                // A concurrent initializer or finalizer installed another database.
                // Finish its teardown outside the lock before trying to publish.
                with_deferred_drops(|| {
                    Radium::store(&zelf.initialized, false, Ordering::Release);
                    zelf.reset_factories(vm);
                    zelf.drop_db(vm)
                })?;
            }
        }
    }

    #[pyclass(with(Constructor, Callable, Initializer), flags(BASETYPE, HAS_WEAKREF))]
    impl Connection {
        fn cached_statement(
            &self,
            sql: PyUtf8StrRef,
            vm: &VirtualMachine,
        ) -> PyResult<Option<StatementLease>> {
            // Check the connection even on a cache hit.
            drop(self.db_lock(vm)?);
            let (cached, generation) = {
                let mut cache = self.statement_cache.lock();
                let statement = cache.entries.get_index_of(sql.as_str()).map(|index| {
                    let last = cache.entries.len() - 1;
                    cache.entries.move_index(index, last);
                    cache.entries.get_index(last).unwrap().1.clone()
                });
                (statement, cache.generation)
            };
            if let Some(statement) = &cached
                && let Some(lease) = StatementLease::acquire(statement.clone())
            {
                return Ok(Some(lease));
            }

            // Preparing may invoke an authorizer. Never hold the cache lock
            // across callbacks, or wait for a statement owned by another cursor.
            let Some(statement) = Statement::new(self, sql.clone(), vm)? else {
                return Ok(None);
            };
            let statement = statement.into_ref(&vm.ctx);
            let lease = StatementLease::acquire(statement.clone()).unwrap();
            let evicted = {
                let mut cache = self.statement_cache.lock();
                if cached.is_none()
                    && cache.generation == generation
                    && cache.capacity > 0
                    && !cache.entries.contains_key(sql.as_str())
                {
                    let evicted = if cache.entries.len() == cache.capacity {
                        cache.entries.shift_remove_index(0)
                    } else {
                        None
                    };
                    cache.entries.insert(sql.as_str().into(), statement);
                    evicted
                } else {
                    None
                }
            };
            drop(evicted);
            Ok(Some(lease))
        }

        fn drop_db(&self, vm: &VirtualMachine) -> PyResult<()> {
            with_deferred_drops(|| Self::drop_db_inner(self, vm))
        }

        fn drop_db_inner(&self, vm: &VirtualMachine) -> PyResult<()> {
            let db = self.db.lock().take();
            let statements = {
                let mut cache = self.statement_cache.lock();
                cache.generation = cache.generation.wrapping_add(1);
                core::mem::take(&mut cache.entries)
            };
            drop(statements);
            let rollback = *self.autocommit.lock() == AutocommitMode::Disabled;
            if let Some(db) = db {
                db.close_callbacks(rollback, vm)?;
            }
            Ok(())
        }

        fn reset_factories(&self, vm: &VirtualMachine) {
            let default_text_factory = PyStr::class(&vm.ctx).to_owned().into_object();
            let _ = unsafe { self.row_factory.swap(None) };
            let _ = unsafe { self.text_factory.swap(default_text_factory) };
        }

        fn initialize_db(args: &ConnectArgs, vm: &VirtualMachine) -> PyResult<PyRc<Sqlite>> {
            let path = args.database.to_cstring(vm)?;
            let db = PyRc::new(Sqlite::from(SqliteRaw::open(path.as_ptr(), args.uri, vm)?));
            #[cfg(feature = "threading")]
            if unsafe { sqlite3_db_mutex(db.db) }.is_null() {
                return Err(vm.new_runtime_error("SQLite connection mutexes are disabled"));
            }
            let timeout = (args.timeout.to_secs_f64() * 1000.0) as c_int;
            db.busy_timeout(timeout);
            if let Some(isolation_level) = &args.isolation_level.0 {
                begin_statement_ptr_from_isolation_level(isolation_level, vm)?;
            }
            if args.autocommit == AutocommitMode::Disabled {
                db._exec(c"BEGIN", vm)?;
            }
            Ok(db)
        }

        fn db_lock(&self, vm: &VirtualMachine) -> PyResult<SqliteOperation> {
            self.check_thread(vm)?;
            self._db_lock(vm)
        }

        fn _db_lock(&self, vm: &VirtualMachine) -> PyResult<SqliteOperation> {
            let guard = self.db.lock();
            if let Some(db) = guard.as_ref() {
                Ok(SqliteOperation(db.clone()))
            } else if self.initialized.load(Ordering::Acquire) {
                Err(new_programming_error(
                    vm,
                    "Cannot operate on a closed database.".to_owned(),
                ))
            } else {
                Err(new_programming_error(
                    vm,
                    "Base Connection.__init__ not called.".to_owned(),
                ))
            }
        }

        fn check_open(&self, vm: &VirtualMachine) -> PyResult<()> {
            self.db_lock(vm).map(drop)
        }

        #[pymethod]
        fn cursor(
            zelf: PyRef<Self>,
            args: CursorArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            zelf.db_lock(vm).map(drop)?;

            let factory = match args.factory {
                OptionalArg::Present(f) => f,
                OptionalArg::Missing => Cursor::class(&vm.ctx).to_owned().into(),
            };

            let cursor = factory.call((zelf.clone(),), vm)?;

            if !cursor.class().fast_issubclass(Cursor::class(&vm.ctx)) {
                return Err(vm.new_type_error(format!(
                    "factory must return a cursor, not {}",
                    cursor.class()
                )));
            }

            if let Some(cursor_ref) = cursor.downcast_ref::<Cursor>()
                && let Some(factory) = zelf.row_factory.load_owned()
            {
                let _ = unsafe { cursor_ref.row_factory.swap(Some(factory)) };
            }

            Ok(cursor)
        }

        #[pymethod]
        fn blobopen(
            zelf: PyRef<Self>,
            args: BlobOpenArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Blob>> {
            with_deferred_drops(|| Self::blobopen_inner(zelf, args, vm))
        }

        fn blobopen_inner(
            zelf: PyRef<Self>,
            args: BlobOpenArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Blob>> {
            let table = args.table.to_cstring(vm)?;
            let column = args.column.to_cstring(vm)?;
            let name = args.name.to_cstring(vm)?;

            let db = zelf.db_lock(vm)?;

            let mut blob = null_mut();
            let ret = db.call(|| unsafe {
                sqlite3_blob_open(
                    db.db,
                    name.as_ptr(),
                    table.as_ptr(),
                    column.as_ptr(),
                    args.rowid,
                    (!args.readonly) as c_int,
                    &mut blob,
                )
            });
            db.check(ret, vm)?;
            let blob = SqliteBlob { blob };
            let blob = Blob {
                connection: zelf,
                inner: PyMutex::new(Some(BlobInner {
                    blob,
                    offset: 0,
                    db: db.0.clone(),
                })),
            };
            Ok(blob.into_ref(&vm.ctx))
        }

        #[pymethod]
        fn close(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            zelf.check_thread(vm)?;
            zelf.drop_db(vm)
        }

        fn is_closed(&self) -> bool {
            self.db.lock().is_none()
        }

        #[pymethod]
        fn commit(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            with_deferred_drops(|| Self::commit_inner(zelf, vm))
        }

        fn commit_inner(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            zelf.end_transaction(c"COMMIT", vm)
        }

        #[pymethod]
        fn rollback(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            with_deferred_drops(|| Self::rollback_inner(zelf, vm))
        }

        fn rollback_inner(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            zelf.end_transaction(c"ROLLBACK", vm)
        }

        fn end_transaction(&self, sql: &CStr, vm: &VirtualMachine) -> PyResult<()> {
            let db = self.db_lock(vm)?;
            let result = {
                let _guard = db.lock();
                let mode = *self.autocommit.lock();
                db.end_transaction(sql, mode)
            };
            result.map_err(|err| err.into_exception(vm))
        }

        fn begin_transaction(&self, db: SqliteRaw, vm: &VirtualMachine) -> PyResult<()> {
            let result = {
                let _guard = db.lock();
                let mode = *self.autocommit.lock();
                if mode == AutocommitMode::Legacy && db.is_autocommit() {
                    db.begin_transaction(self.isolation_level.deref().map(|x| x.to_owned()))
                } else {
                    Ok(())
                }
            };
            result.map_err(|err| err.into_exception(vm))
        }

        fn commit_before_script(&self, db: SqliteRaw, vm: &VirtualMachine) -> PyResult<()> {
            let result = {
                let _guard = db.lock();
                let mode = *self.autocommit.lock();
                if mode == AutocommitMode::Legacy {
                    db.end_transaction(c"COMMIT", mode)
                } else {
                    Ok(())
                }
            };
            result.map_err(|err| err.into_exception(vm))
        }

        #[pymethod]
        fn execute(
            zelf: PyRef<Self>,
            sql: PyUtf8StrRef,
            parameters: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Cursor>> {
            let row_factory = zelf.row_factory.load_owned();
            let cursor = Cursor::new(zelf, row_factory, vm).into_ref(&vm.ctx);
            Cursor::execute(cursor, sql, parameters, vm)
        }

        #[pymethod]
        fn executemany(
            zelf: PyRef<Self>,
            sql: PyUtf8StrRef,
            seq_of_params: ArgIterable,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Cursor>> {
            let row_factory = zelf.row_factory.load_owned();
            let cursor = Cursor::new(zelf, row_factory, vm).into_ref(&vm.ctx);
            Cursor::executemany(cursor, sql, seq_of_params, vm)
        }

        #[pymethod]
        fn executescript(
            zelf: PyRef<Self>,
            script: PyUtf8StrRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Cursor>> {
            let row_factory = zelf.row_factory.load_owned();
            Cursor::executescript(
                Cursor::new(zelf, row_factory, vm).into_ref(&vm.ctx),
                script,
                vm,
            )
        }

        #[pymethod]
        fn backup(zelf: &Py<Self>, args: BackupArgs, vm: &VirtualMachine) -> PyResult<()> {
            with_deferred_drops(|| Self::backup_inner(zelf, args, vm))
        }

        fn backup_inner(zelf: &Py<Self>, args: BackupArgs, vm: &VirtualMachine) -> PyResult<()> {
            let BackupArgs {
                target,
                pages,
                progress,
                name,
                sleep,
            } = args;
            if zelf.is(&target) {
                return Err(vm.new_value_error("target cannot be the same connection instance"));
            }

            if let Some(progress) = &progress
                && !progress.is_callable()
            {
                return Err(vm.new_type_error("progress argument must be a callable"));
            }

            let pages = if pages == 0 { -1 } else { pages };

            let name_cstring;
            let name_ptr = if let Some(name) = &name {
                name_cstring = name.to_cstring(vm)?;
                name_cstring.as_ptr()
            } else {
                c"main".as_ptr()
            };

            let sleep_ms = (sleep * 1000.0) as c_int;

            let db = zelf.db_lock(vm)?;
            let target_db = target.db_lock(vm)?;

            let handle = {
                // Match SQLite's source-before-destination lock order, and retain
                // the destination error before another operation can replace it.
                let _source_guard = db.lock();
                let _target_guard = target_db.lock();
                let handle =
                    unsafe { sqlite3_backup_init(target_db.db, c"main".as_ptr(), db.db, name_ptr) };
                if handle.is_null() {
                    Err(target_db.error_snapshot())
                } else {
                    Ok(handle)
                }
            }
            .map_err(|err| err.into_exception(vm))?;

            loop {
                let ret = unsafe { sqlite3_backup_step(handle, pages) };

                if let Some(progress) = &progress {
                    let remaining = unsafe { sqlite3_backup_remaining(handle) };
                    let pagecount = unsafe { sqlite3_backup_pagecount(handle) };
                    if let Err(err) = progress.call((ret, remaining, pagecount), vm) {
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

            let result = {
                let _source_guard = db.lock();
                target_db.call(|| unsafe { sqlite3_backup_finish(handle) })
            };
            target_db.check(result, vm)
        }

        #[pymethod]
        fn create_function(
            zelf: &Py<Self>,
            args: CreateFunctionArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            with_deferred_drops(|| Self::create_function_inner(zelf, args, vm))
        }

        fn create_function_inner(
            zelf: &Py<Self>,
            args: CreateFunctionArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let name = args.name.to_cstring(vm)?;
            let flags = if args.deterministic {
                SQLITE_UTF8 | SQLITE_DETERMINISTIC
            } else {
                SQLITE_UTF8
            };
            let db = zelf.db_lock(vm)?;
            check_num_params(&db, args.narg, "narg", vm)?;
            let Some(data) = CallbackData::new(args.func, vm) else {
                return db.create_function(
                    &name,
                    args.narg,
                    flags,
                    null_mut(),
                    None,
                    None,
                    None,
                    None,
                    vm,
                );
            };

            db.create_function(
                &name,
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
        fn create_aggregate(
            zelf: &Py<Self>,
            args: CreateAggregateArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            with_deferred_drops(|| Self::create_aggregate_inner(zelf, args, vm))
        }

        fn create_aggregate_inner(
            zelf: &Py<Self>,
            args: CreateAggregateArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let name = args.name.to_cstring(vm)?;
            let db = zelf.db_lock(vm)?;
            check_num_params(&db, args.narg, "n_arg", vm)?;
            let Some(data) = CallbackData::new(args.aggregate_class, vm) else {
                return db.create_function(
                    &name,
                    args.narg,
                    SQLITE_UTF8,
                    null_mut(),
                    None,
                    None,
                    None,
                    None,
                    vm,
                );
            };

            db.create_function(
                &name,
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
            zelf: &Py<Self>,
            name: PyUtf8StrRef,
            callable: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            with_deferred_drops(|| Self::create_collation_inner(zelf, name, callable, vm))
        }

        fn create_collation_inner(
            zelf: &Py<Self>,
            name: PyUtf8StrRef,
            callable: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let name = name.to_cstring(vm)?;
            let db = zelf.db_lock(vm)?;
            let Some(data) = CallbackData::new(callable.clone(), vm) else {
                unsafe {
                    sqlite3_create_collation_v2(
                        db.db,
                        name.as_ptr(),
                        SQLITE_UTF8,
                        null_mut(),
                        None,
                        None,
                    );
                }
                return Ok(());
            };
            let data = Box::into_raw(Box::new(data));

            if !callable.is_callable() {
                return Err(vm.new_type_error("parameter must be callable"));
            }

            let ret = db.call(|| unsafe {
                sqlite3_create_collation_v2(
                    db.db,
                    name.as_ptr(),
                    SQLITE_UTF8,
                    data.cast(),
                    Some(CallbackData::collation_callback),
                    Some(CallbackData::destructor),
                )
            });

            db.check(ret, vm).inspect_err(|_| {
                // create_collation do not call destructor if error occur
                let _ = unsafe { Box::from_raw(data) };
            })
        }

        #[pymethod]
        fn create_window_function(
            zelf: &Py<Self>,
            name: PyStrRef,
            narg: c_int,
            aggregate_class: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            with_deferred_drops(|| {
                Self::create_window_function_inner(zelf, name, narg, aggregate_class, vm)
            })
        }

        fn create_window_function_inner(
            zelf: &Py<Self>,
            name: PyStrRef,
            narg: c_int,
            aggregate_class: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let name = name.to_cstring(vm)?;
            let db = zelf.db_lock(vm)?;
            check_num_params(&db, narg, "num_params", vm)?;
            let Some(data) = CallbackData::new(aggregate_class, vm) else {
                unsafe {
                    sqlite3_create_window_function(
                        db.db,
                        name.as_ptr(),
                        narg,
                        SQLITE_UTF8,
                        null_mut(),
                        None,
                        None,
                        None,
                        None,
                        None,
                    )
                };
                return Ok(());
            };

            let ret = db.call(|| unsafe {
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
            });
            db.check(ret, vm)
                .map_err(|_| new_programming_error(vm, "Error creating window function".to_owned()))
        }

        #[pymethod]
        fn set_authorizer(
            zelf: &Py<Self>,
            callable: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            zelf.set_callback(callable, CallbackKind::Authorizer, vm)
        }

        #[pymethod]
        fn set_trace_callback(
            zelf: &Py<Self>,
            callable: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            zelf.set_callback(callable, CallbackKind::Trace, vm)
        }

        #[pymethod]
        fn set_progress_handler(
            zelf: &Py<Self>,
            callable: PyObjectRef,
            n: c_int,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            zelf.set_callback(callable, CallbackKind::Progress(n), vm)
        }

        fn set_callback(
            &self,
            callable: PyObjectRef,
            kind: CallbackKind,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            with_deferred_drops(|| {
                let db = self.db_lock(vm)?;
                let mut data = CallbackData::new(callable, vm).map(PyRc::new);
                let pointer = data
                    .as_ref()
                    .map_or(null_mut(), |data| PyRc::as_ptr(data).cast_mut());
                let (closed, result, previous) = {
                    let _guard = db.lock();
                    let mut callbacks = db.callbacks.lock();
                    if callbacks.closed {
                        (true, Ok(SQLITE_OK), None)
                    } else {
                        let result = db.call(|| kind.register(db.raw, pointer));
                        let previous = if result.is_ok() {
                            core::mem::replace(kind.slot(&mut callbacks), data.take())
                        } else {
                            None
                        };
                        (false, result, previous)
                    }
                };
                // Both the native mutex and the slots mutex must be released
                // before a displaced callable can run __del__ or weakref callbacks.
                drop((previous, data));
                if closed {
                    return Err(new_programming_error(
                        vm,
                        "Cannot operate on a closed database.".to_owned(),
                    ));
                }
                db.check(result, vm).map_err(|error| {
                    if matches!(kind, CallbackKind::Authorizer) {
                        new_operational_error(vm, "Error setting authorizer callback".to_owned())
                    } else {
                        error
                    }
                })
            })
        }

        #[pymethod]
        fn iterdump(zelf: PyRef<Self>, args: IterDumpArgs, vm: &VirtualMachine) -> PyResult {
            let from_list = PyTuple::new_ref_typed(vec![vm.ctx.new_str("_iterdump")], &vm.ctx);
            let module = vm.import_from("sqlite3.dump", from_list, 0)?;
            let func = module.get_attr("_iterdump", vm)?;
            let filter: PyObjectRef = args
                .filter
                .map_or_else(|| vm.ctx.none(), |filter| filter.into());
            let kwargs = core::iter::once(("filter", filter)).collect::<KwArgs>();
            func.call(FuncArgs::new(vec![zelf.into()], kwargs), vm)
        }

        #[pymethod]
        fn interrupt(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            // DO NOT check thread safety
            zelf._db_lock(vm).map(|x| x.interrupt())
        }

        #[pymethod]
        fn getlimit(zelf: &Py<Self>, category: c_int, vm: &VirtualMachine) -> PyResult<c_int> {
            zelf.db_lock(vm)?.limit(category, -1, vm)
        }

        #[pymethod]
        fn setlimit(
            zelf: &Py<Self>,
            category: c_int,
            limit: c_int,
            vm: &VirtualMachine,
        ) -> PyResult<c_int> {
            zelf.db_lock(vm)?.limit(category, limit, vm)
        }

        #[pymethod]
        fn setconfig(
            zelf: &Py<Self>,
            op: c_int,
            enable: OptionalArg<bool>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let db = zelf.db_lock(vm)?;
            if !is_int_dbconfig(op) {
                return Err(vm.new_value_error(format!("unknown config 'op': {op}")));
            }
            let enable = enable.unwrap_or(true) as c_int;
            let mut actual: c_int = 0;
            let rc = db.call(|| unsafe { sqlite3_db_config(db.db, op, enable, &mut actual) });
            db.check(rc, vm)?;
            if enable != actual {
                return Err(new_operational_error(vm, "Unable to set config".to_owned()));
            }
            Ok(())
        }

        #[pymethod]
        fn getconfig(zelf: &Py<Self>, op: c_int, vm: &VirtualMachine) -> PyResult<bool> {
            let db = zelf.db_lock(vm)?;
            if !is_int_dbconfig(op) {
                return Err(vm.new_value_error(format!("unknown config 'op': {op}")));
            }
            let mut current: c_int = 0;
            let rc = db.call(|| unsafe { sqlite3_db_config(db.db, op, -1, &mut current) });
            db.check(rc, vm)?;
            Ok(current != 0)
        }

        #[pymethod]
        fn __enter__(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pymethod]
        fn __exit__(
            zelf: &Py<Self>,
            cls: PyObjectRef,
            exc: PyObjectRef,
            tb: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if vm.is_none(&cls) && vm.is_none(&exc) && vm.is_none(&tb) {
                Self::commit(zelf, vm)
            } else {
                Self::rollback(zelf, vm)
            }
        }

        #[pygetset]
        fn isolation_level(zelf: &Py<Self>) -> Option<PyStrRef> {
            zelf.isolation_level.deref().map(|x| x.to_owned())
        }
        #[pygetset(setter)]
        fn set_isolation_level(
            zelf: &Py<Self>,
            value: PySetterValue<IsolationLevelArg>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match value {
                PySetterValue::Assign(IsolationLevelArg(value)) => {
                    if let Some(val_str) = &value {
                        begin_statement_ptr_from_isolation_level(val_str, vm)?;
                    }

                    // If setting isolation_level to None (auto-commit mode), commit any pending transaction
                    if value.is_none() {
                        Self::commit(zelf, vm)?;
                    }
                    let _ = unsafe { zelf.isolation_level.swap(value) };
                    Ok(())
                }
                PySetterValue::Delete => {
                    Err(vm.new_attribute_error("'isolation_level' attribute cannot be deleted"))
                }
            }
        }

        #[pygetset]
        fn autocommit(zelf: &Py<Self>, vm: &VirtualMachine) -> PyObjectRef {
            let mode = *zelf.autocommit.lock();
            match mode {
                AutocommitMode::Enabled => vm.ctx.true_value.clone().into(),
                AutocommitMode::Disabled => vm.ctx.false_value.clone().into(),
                AutocommitMode::Legacy => vm.ctx.new_int(LEGACY_TRANSACTION_CONTROL).into(),
            }
        }
        #[pygetset(setter)]
        fn set_autocommit(zelf: &Py<Self>, val: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            with_deferred_drops(|| Self::set_autocommit_inner(zelf, val, vm))
        }

        fn set_autocommit_inner(
            zelf: &Py<Self>,
            val: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let mode = AutocommitMode::try_from_borrowed_object(vm, &val)?;
            let db = zelf.db_lock(vm)?;
            let result = {
                // Publish the Python mode and change SQLite's transaction state
                // under the same mutex used by statement execution.
                let _guard = db.lock();
                *zelf.autocommit.lock() = mode;

                // Handle transaction state based on mode change
                match mode {
                    AutocommitMode::Enabled => {
                        // If there's a pending transaction, commit it
                        db.end_transaction(c"COMMIT", AutocommitMode::Legacy)
                    }
                    AutocommitMode::Disabled => {
                        // If not in a transaction, begin one
                        if db.is_autocommit() {
                            db.exec(c"BEGIN")
                        } else {
                            Ok(())
                        }
                    }
                    AutocommitMode::Legacy => {
                        // Legacy mode doesn't change transaction state
                        Ok(())
                    }
                }
            };
            result.map_err(|err| err.into_exception(vm))
        }

        #[pygetset]
        fn text_factory(zelf: &Py<Self>) -> PyObjectRef {
            zelf.text_factory.to_owned()
        }
        #[pygetset(setter)]
        fn set_text_factory(
            zelf: &Py<Self>,
            val: PySetterValue<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match val {
                PySetterValue::Assign(val) => {
                    let _ = unsafe { zelf.text_factory.swap(val) };
                    Ok(())
                }
                PySetterValue::Delete => {
                    Err(vm.new_attribute_error("cannot delete text_factory attribute"))
                }
            }
        }

        #[pygetset]
        fn row_factory(zelf: &Py<Self>) -> Option<PyObjectRef> {
            zelf.row_factory.load_owned()
        }
        #[pygetset(setter)]
        fn set_row_factory(
            zelf: &Py<Self>,
            val: PySetterValue<Option<PyObjectRef>>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match val {
                PySetterValue::Assign(val) => {
                    let _ = unsafe { zelf.row_factory.swap(val) };
                    Ok(())
                }
                PySetterValue::Delete => {
                    Err(vm.new_attribute_error("cannot delete row_factory attribute"))
                }
            }
        }

        fn check_thread(&self, vm: &VirtualMachine) -> PyResult<()> {
            if self.check_same_thread.load(Ordering::Relaxed) {
                let creator_id = *self.thread_ident.lock();
                if std::thread::current().id() != creator_id {
                    return Err(new_programming_error(
                        vm,
                        "SQLite objects created in a thread can only be used in that same thread."
                            .to_owned(),
                    ));
                }
            }
            Ok(())
        }

        #[pygetset]
        fn in_transaction(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<bool> {
            let db = zelf._db_lock(vm)?;
            let _guard = db.lock();
            Ok(!db.is_autocommit())
        }

        #[pygetset]
        fn total_changes(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<c_int> {
            let db = zelf._db_lock(vm)?;
            let _guard = db.lock();
            Ok(db.total_changes())
        }

        #[pygetset(name = "Warning")]
        fn exc_warning(_zelf: &Py<Self>) -> PyTypeRef {
            warning_type().to_owned()
        }
        #[pygetset(name = "Error")]
        fn exc_error(_zelf: &Py<Self>) -> PyTypeRef {
            error_type().to_owned()
        }
        #[pygetset(name = "InterfaceError")]
        fn exc_interface_error(_zelf: &Py<Self>) -> PyTypeRef {
            interface_error_type().to_owned()
        }
        #[pygetset(name = "DatabaseError")]
        fn exc_database_error(_zelf: &Py<Self>) -> PyTypeRef {
            database_error_type().to_owned()
        }
        #[pygetset(name = "DataError")]
        fn exc_data_error(_zelf: &Py<Self>) -> PyTypeRef {
            data_error_type().to_owned()
        }
        #[pygetset(name = "OperationalError")]
        fn exc_operational_error(_zelf: &Py<Self>) -> PyTypeRef {
            operational_error_type().to_owned()
        }
        #[pygetset(name = "IntegrityError")]
        fn exc_integrity_error(_zelf: &Py<Self>) -> PyTypeRef {
            integrity_error_type().to_owned()
        }
        #[pygetset(name = "InternalError")]
        fn exc_internal_error(_zelf: &Py<Self>) -> PyTypeRef {
            internal_error_type().to_owned()
        }
        #[pygetset(name = "ProgrammingError")]
        fn exc_programming_error(_zelf: &Py<Self>) -> PyTypeRef {
            programming_error_type().to_owned()
        }
        #[pygetset(name = "NotSupportedError")]
        fn exc_not_supported_error(_zelf: &Py<Self>) -> PyTypeRef {
            not_supported_error_type().to_owned()
        }
    }

    #[pyattr]
    #[pyclass(name, traverse)]
    #[derive(Debug, PyPayload)]
    struct Cursor {
        connection: PyRef<Connection>,
        #[pytraverse(skip)]
        arraysize: PyAtomic<c_int>,
        #[pytraverse(skip)]
        row_factory: PyAtomicRef<Option<PyObject>>,
        inner: PyMutex<Option<CursorInner>>,
    }

    #[derive(Debug, Traverse)]
    struct CursorInner {
        description: Option<PyTupleRef>,
        row_cast_map: Vec<Option<PyObjectRef>>,
        #[pytraverse(skip)]
        lastrowid: i64,
        #[pytraverse(skip)]
        rowcount: i64,
        statement: Option<StatementLease>,
        #[pytraverse(skip)]
        closed: bool,
    }

    #[derive(FromArgs)]
    struct FetchManyArgs {
        #[pyarg(any, name = "size", optional)]
        size: Option<c_int>,
    }

    #[pyclass(
        with(Constructor, Initializer, IterNext, Iterable),
        flags(BASETYPE, HAS_WEAKREF)
    )]
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
                    closed: false,
                })),
            }
        }

        fn new_uninitialized(connection: PyRef<Connection>, _vm: &VirtualMachine) -> Self {
            Self {
                connection,
                arraysize: Radium::new(1),
                row_factory: PyAtomicRef::from(None),
                inner: PyMutex::from(None),
            }
        }

        fn check_cursor_state(inner: Option<&CursorInner>, vm: &VirtualMachine) -> PyResult<()> {
            match inner {
                Some(inner) if inner.closed => Err(new_programming_error(
                    vm,
                    "Cannot operate on a closed cursor.".to_owned(),
                )),
                Some(_) => Ok(()),
                None => Err(new_programming_error(
                    vm,
                    "Base Cursor.__init__ not called.".to_owned(),
                )),
            }
        }

        fn inner(&self, vm: &VirtualMachine) -> PyResult<PyMappedMutexGuard<'_, CursorInner>> {
            let guard = self.inner.lock();
            Self::check_cursor_state(guard.as_ref(), vm)?;
            Ok(PyMutexGuard::map(guard, |x| unsafe {
                x.as_mut().unwrap_unchecked()
            }))
        }

        /// Check if cursor is valid without retaining the lock.
        /// Use this when you only need to verify the cursor state but don't need to modify it.
        fn check_cursor_valid(&self, vm: &VirtualMachine) -> PyResult<()> {
            let guard = self.inner.lock();
            Self::check_cursor_state(guard.as_ref(), vm)
        }

        #[pymethod]
        fn execute(
            zelf: PyRef<Self>,
            sql: PyUtf8StrRef,
            parameters: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            with_deferred_drops(|| Self::execute_inner(zelf, sql, parameters, vm))
        }

        fn execute_inner(
            zelf: PyRef<Self>,
            sql: PyUtf8StrRef,
            parameters: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let mut inner = zelf.inner(vm)?;

            if let Some(stmt) = inner.statement.take() {
                stmt.lock().reset();
            }

            let Some(stmt) = zelf.connection.cached_statement(sql, vm)? else {
                drop(inner);
                return Ok(zelf);
            };

            inner.rowcount = if stmt.is_dml { 0 } else { -1 };

            let db = stmt.database(&zelf.connection, vm)?;

            // Only legacy transaction control starts implicit DML transactions.
            if stmt.is_dml {
                zelf.connection.begin_transaction(db.raw, vm)?;
            }

            let st = stmt.lock();
            let params_needed = st.bind_parameter_count();

            if let OptionalArg::Present(parameters) = parameters {
                st.bind_parameters(&parameters, vm)?;
            } else if params_needed > 0 {
                let msg = format!(
                    "Incorrect number of bindings supplied. The current statement uses {params_needed}, and 0 were supplied."
                );
                return Err(new_programming_error(vm, msg));
            }

            zelf.connection.check_open(vm)?;
            let ret = st.step();
            zelf.connection.check_open(vm)?;
            let ret = ret.map_err(|error| {
                unsafe { user_function_exception().swap(None) }
                    .unwrap_or_else(|| error.into_exception(vm))
            })?;

            inner.row_cast_map = zelf.build_row_cast_map(&st, vm)?;

            let detect_types = zelf.connection.detect_types.load(Ordering::Relaxed);
            inner.description = st.columns_description(detect_types, vm)?;

            if ret.has_row {
                drop(st);
                inner.statement = Some(stmt);
            } else {
                st.reset();
                drop(st);
                if stmt.is_dml {
                    inner.rowcount += ret.changes as i64;
                }
            }

            inner.lastrowid = ret.lastrowid;

            drop(inner);
            Ok(zelf)
        }

        #[pymethod]
        fn executemany(
            zelf: PyRef<Self>,
            sql: PyUtf8StrRef,
            seq_of_params: ArgIterable,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            with_deferred_drops(|| Self::executemany_inner(zelf, sql, seq_of_params, vm))
        }

        fn executemany_inner(
            zelf: PyRef<Self>,
            sql: PyUtf8StrRef,
            seq_of_params: ArgIterable,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let mut inner = zelf.inner(vm)?;

            if let Some(stmt) = inner.statement.take() {
                stmt.lock().reset();
            }

            let Some(stmt) = zelf.connection.cached_statement(sql, vm)? else {
                drop(inner);
                return Ok(zelf);
            };

            let db = stmt.database(&zelf.connection, vm)?;
            let st = stmt.lock();

            if st.readonly() {
                return Err(new_programming_error(
                    vm,
                    "executemany() can only execute DML statements.".to_owned(),
                ));
            }

            let detect_types = zelf.connection.detect_types.load(Ordering::Relaxed);
            inner.description = st.columns_description(detect_types, vm)?;

            inner.rowcount = if stmt.is_dml { 0 } else { -1 };

            // Only legacy transaction control starts implicit DML transactions.
            if stmt.is_dml {
                zelf.connection.begin_transaction(db.raw, vm)?;
            }

            let iter = seq_of_params.iter(vm)?;
            for params in iter {
                let params = params?;
                zelf.connection.check_open(vm)?;
                st.bind_parameters(&params, vm)?;
                zelf.connection.check_open(vm)?;

                let ret = st.step_checked(vm);
                zelf.connection.check_open(vm)?;
                let ret = ret?;
                if !ret.has_row {
                    if stmt.is_dml {
                        inner.rowcount += ret.changes as i64;
                    }
                    st.reset();
                }

                // if let Some(exc) = unsafe { user_function_exception().swap(None) } {
                //     return Err(exc);
                // }
            }

            zelf.connection.check_open(vm)?;
            if st.busy() {
                drop(st);
                inner.statement = Some(stmt);
            }

            drop(inner);
            Ok(zelf)
        }

        #[pymethod]
        fn executescript(
            zelf: PyRef<Self>,
            script: PyUtf8StrRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            with_deferred_drops(|| Self::executescript_inner(zelf, script, vm))
        }

        fn executescript_inner(
            zelf: PyRef<Self>,
            script: PyUtf8StrRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            zelf.check_cursor_valid(vm)?;

            let db = zelf.connection.db_lock(vm)?;

            db.sql_limit(script.byte_len(), vm)?;

            let result = zelf.connection.commit_before_script(db.raw, vm);
            zelf.connection.check_open(vm)?;
            result?;

            let script = script.to_cstring(vm)?;
            let mut sql = script.as_c_str();
            let mut tail = null();

            loop {
                let statement = db.prepare(sql, &mut tail, vm);
                zelf.connection.check_open(vm)?;
                let Some(st) = statement? else {
                    break;
                };
                loop {
                    let result = st.step_checked(vm);
                    zelf.connection.check_open(vm)?;
                    if !result?.has_row {
                        break;
                    }
                }
                // SAFETY: SQLite advances the pointer w.r.t. `sql`, and `sql` is a valid CStr
                // without any interior NULs.
                sql = unsafe { CStr::from_ptr(tail) };
            }

            drop(db);
            Ok(zelf)
        }

        #[pymethod]
        fn fetchone(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            Self::next(zelf, vm).map(|x| match x {
                PyIterReturn::Return(row) => row,
                PyIterReturn::StopIteration(_) => vm.ctx.none(),
            })
        }

        #[pymethod]
        fn fetchmany(
            zelf: &Py<Self>,
            args: FetchManyArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<PyObjectRef>> {
            let max_rows = match args.size {
                Some(size) => {
                    if size < 0 {
                        return Err(vm.new_value_error("fetchmany may not be negative"));
                    }

                    size
                }
                None => zelf.arraysize.load(Ordering::Relaxed),
            };

            let mut list = vec![];
            let mut remaining = max_rows;
            while remaining > 0 {
                match Self::next(zelf, vm)? {
                    PyIterReturn::Return(row) => {
                        list.push(row);
                        remaining -= 1;
                    }
                    PyIterReturn::StopIteration(_) => break,
                }
            }
            Ok(list)
        }

        #[pymethod]
        fn fetchall(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            let mut list = vec![];
            while let PyIterReturn::Return(row) = Self::next(zelf, vm)? {
                list.push(row);
            }
            Ok(list)
        }

        #[pymethod]
        fn close(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            with_deferred_drops(|| Self::close_inner(zelf, vm))
        }

        fn close_inner(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            // Check if __init__ was called
            let mut guard = zelf.inner.lock();

            let Some(inner) = guard.as_mut() else {
                return Err(new_programming_error(
                    vm,
                    "Base Cursor.__init__ not called.".to_owned(),
                ));
            };

            drop(inner.statement.take());
            inner.closed = true;

            Ok(())
        }

        #[pymethod]
        fn setinputsizes(_zelf: &Py<Self>, _sizes: PyObjectRef) {}

        #[pymethod]
        fn setoutputsize(_zelf: &Py<Self>, _size: PyObjectRef, _column: OptionalArg<PyObjectRef>) {}

        #[pygetset]
        fn connection(zelf: &Py<Self>) -> PyRef<Connection> {
            zelf.connection.clone()
        }

        #[pygetset]
        fn lastrowid(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<i64> {
            zelf.inner(vm).map(|x| x.lastrowid)
        }

        #[pygetset]
        fn rowcount(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<i64> {
            zelf.inner(vm).map(|x| x.rowcount)
        }

        #[pygetset]
        fn description(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Option<PyTupleRef>> {
            zelf.inner(vm).map(|x| x.description.clone())
        }

        #[pygetset]
        fn arraysize(zelf: &Py<Self>) -> c_int {
            zelf.arraysize.load(Ordering::Relaxed)
        }

        #[pygetset(setter)]
        fn set_arraysize(zelf: &Py<Self>, val: c_int, vm: &VirtualMachine) -> PyResult<()> {
            if val < 0 {
                return Err(vm.new_value_error("arraysize may not be negative"));
            }

            zelf.arraysize.store(val, Ordering::Relaxed);

            Ok(())
        }

        #[pygetset]
        fn row_factory(zelf: &Py<Self>) -> Option<PyObjectRef> {
            zelf.row_factory.load_owned()
        }

        #[pygetset(setter)]
        fn set_row_factory(zelf: &Py<Self>, val: Option<PyObjectRef>) {
            let _ = unsafe { zelf.row_factory.swap(val) };
        }

        fn build_row_cast_map(
            &self,
            st: &SqliteStatementRaw,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<Option<PyObjectRef>>> {
            let detect_types = self.connection.detect_types.load(Ordering::Relaxed);
            if detect_types == 0 {
                return Ok(vec![]);
            }

            let mut cast_map = vec![];
            let num_cols = st.column_count();

            for i in 0..num_cols {
                if detect_types & PARSE_COLNAMES != 0 {
                    let col_name = st.column_name(i);
                    let col_name = ptr_to_str(col_name, vm)?;
                    let col_name = col_name
                        .chars()
                        .skip_while(|&x| x != '[')
                        .skip(1)
                        .take_while(|&x| x != ']')
                        .flat_map(|x| x.to_uppercase())
                        .collect::<Box<str>>();
                    if let Some(converter) = converters().get_item_opt(&*col_name, vm)? {
                        cast_map.push(Some(converter));
                        continue;
                    }
                }
                if detect_types & PARSE_DECLTYPES != 0 {
                    let decltype = st.column_decltype(i);
                    let decltype = ptr_to_str(decltype, vm)?;
                    if let Some(decltype) = decltype.split_terminator(&[' ', '(']).next() {
                        let decltype = decltype.to_uppercase();
                        if let Some(converter) = converters().get_item_opt(&decltype, vm)? {
                            cast_map.push(Some(converter));
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

        fn py_new(
            _cls: &Py<PyType>,
            (connection,): Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            Ok(Self::new_uninitialized(connection, vm))
        }
    }

    impl Initializer for Cursor {
        type Args = PyRef<Connection>;

        fn init(zelf: &Py<Self>, _connection: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            let mut guard = zelf.inner.lock();
            if guard.is_some() {
                // Already initialized (e.g., from a call to super().__init__)
                return Ok(());
            }
            *guard = Some(CursorInner {
                description: None,
                row_cast_map: vec![],
                lastrowid: -1,
                rowcount: -1,
                statement: None,
                closed: false,
            });
            Ok(())
        }
    }

    impl SelfIter for Cursor {}
    impl IterNext for Cursor {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            with_deferred_drops(|| Self::next_inner(zelf, vm))
        }
    }

    impl Cursor {
        fn next_inner(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            // Check if connection is closed first, and if so, clear statement to release file lock
            if zelf.connection.is_closed() {
                let mut guard = zelf.inner.lock();
                if let Some(stmt) = guard.as_mut().and_then(|inner| inner.statement.take()) {
                    stmt.lock().reset();
                }
                return Err(new_programming_error(
                    vm,
                    "Cannot operate on a closed database.".to_owned(),
                ));
            }

            let mut inner = zelf.inner(vm)?;
            let Some(stmt) = &inner.statement else {
                return Ok(PyIterReturn::StopIteration(None));
            };
            let db = stmt.database(&zelf.connection, vm)?;
            let st = stmt.lock();
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
                            core::slice::from_raw_parts(blob.cast::<u8>(), nbytes as usize)
                        };
                        let blob = vm.ctx.new_bytes(blob.to_vec());
                        let value = converter.call((blob,), vm)?;
                        zelf.connection.check_open(vm)?;
                        value
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

                            if text_factory.is(PyStr::class(&vm.ctx)) {
                                let text = String::from_utf8(text).map_err(|err| {
                                    let col_name = st.column_name(i);
                                    let col_name_str = ptr_to_str(col_name, vm).unwrap_or("?");
                                    let valid_up_to = err.utf8_error().valid_up_to();
                                    let text_prefix = String::from_utf8_lossy(&err.as_bytes()[..valid_up_to]);
                                    let msg = format!(
                                        "Could not decode to UTF-8 column '{col_name_str}' with text '{text_prefix}'"
                                    );
                                    new_operational_error(vm, msg)
                                })?;
                                vm.ctx.new_str(text).into()
                            } else if text_factory.is(PyBytes::class(&vm.ctx)) {
                                vm.ctx.new_bytes(text).into()
                            } else if text_factory.is(PyByteArray::class(&vm.ctx)) {
                                PyByteArray::from(text).into_ref(&vm.ctx).into()
                            } else {
                                let bytes = vm.ctx.new_bytes(text);
                                let value = text_factory.call((bytes,), vm)?;
                                zelf.connection.check_open(vm)?;
                                value
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

            let ret = st.step_checked(vm);
            zelf.connection.check_open(vm)?;
            let ret = ret?;
            if !ret.has_row {
                st.reset();
                drop(st);
                if stmt.is_dml {
                    inner.rowcount = ret.changes as i64;
                }
                inner.statement = None;
            } else {
                drop(st);
            }

            drop(db);
            drop(inner);

            let row = vm.ctx.new_tuple(row);

            if let Some(row_factory) = zelf.row_factory.load_owned() {
                row_factory
                    .call((zelf.to_owned(), row), vm)
                    .map(PyIterReturn::Return)
            } else {
                Ok(PyIterReturn::Return(row.into()))
            }
        }
    }

    #[pyattr]
    #[pyclass(name, traverse)]
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
        fn keys(zelf: &Py<Self>, _vm: &VirtualMachine) -> Vec<PyObjectRef> {
            zelf.description
                .as_slice()
                .iter()
                .map(|x| x.downcast_ref::<PyTuple>().unwrap().as_slice()[0].clone())
                .collect()
        }

        fn subscript(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
            if let Some(i) = needle.downcast_ref::<PyInt>() {
                let i = i.try_to_primitive::<isize>(vm).map_err(|_| {
                    vm.new_index_error("cannot fit 'int' into an index-sized integer")
                })?;
                self.data.as_slice().getitem_by_index(vm, i)
            } else if let Some(name) = needle.downcast_ref::<PyStr>() {
                for (obj, i) in self.description.as_slice().iter().zip(0..) {
                    let obj = &obj.downcast_ref::<PyTuple>().unwrap().as_slice()[0];
                    let Some(obj) = obj.downcast_ref::<PyStr>() else {
                        break;
                    };
                    let name = name.expect_str();
                    let obj = obj.expect_str();

                    if name == obj
                        || (name.is_ascii() && obj.is_ascii() && name.eq_ignore_ascii_case(obj))
                    {
                        return self.data.as_slice().getitem_by_index(vm, i);
                    }
                }
                Err(vm.new_index_error(format!("No item with key '{}'", name.to_string_lossy())))
            } else if let Some(slice) = needle.downcast_ref::<PySlice>() {
                let list = self
                    .data
                    .as_slice()
                    .getitem_by_slice(vm, slice.to_saturated(vm)?)?;
                Ok(vm.ctx.new_tuple(list).into())
            } else {
                Err(vm.new_index_error("Index must be int or string"))
            }
        }
    }

    impl Constructor for Row {
        type Args = (PyRef<Cursor>, PyTupleRef);

        fn py_new(
            _cls: &Py<PyType>,
            (cursor, data): Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let description = cursor
                .inner(vm)?
                .description
                .clone()
                .unwrap_or_else(|| vm.ctx.empty_tuple.clone());

            Ok(Self { data, description })
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
                if let Some(other) = other.downcast_ref::<Self>() {
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
            static AS_MAPPING: std::sync::LazyLock<PyMappingMethods> =
                std::sync::LazyLock::new(|| PyMappingMethods {
                    length: atomic_func!(|mapping, _vm| Ok(Row::mapping_downcast(mapping)
                        .data
                        .as_slice()
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
            static AS_SEQUENCE: std::sync::LazyLock<PySequenceMethods> =
                std::sync::LazyLock::new(|| PySequenceMethods {
                    length: atomic_func!(|seq, _vm| Ok(Row::sequence_downcast(seq)
                        .data
                        .as_slice()
                        .len())),
                    item: atomic_func!(|seq, i, vm| Row::sequence_downcast(seq)
                        .data
                        .as_slice()
                        .getitem_by_index(vm, i)),
                    ..PySequenceMethods::NOT_IMPLEMENTED
                });
            &AS_SEQUENCE
        }
    }

    #[pyattr]
    #[pyclass(module = "sqlite3", name = "Blob", traverse)]
    #[derive(Debug, PyPayload)]
    struct Blob {
        connection: PyRef<Connection>,
        #[pytraverse(skip)]
        inner: PyMutex<Option<BlobInner>>,
    }

    struct BlobInner {
        blob: SqliteBlob,
        offset: c_int,
        db: PyRc<Sqlite>,
    }

    impl Debug for BlobInner {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("BlobInner")
                .field("blob", &self.blob)
                .field("offset", &self.offset)
                .finish_non_exhaustive()
        }
    }

    impl BlobInner {
        fn check(&self, ret: c_int, vm: &VirtualMachine) -> PyResult<()> {
            if ret == SQLITE_OK {
                Ok(())
            } else {
                Err(self.db.error_extended(vm))
            }
        }
    }

    impl Drop for BlobInner {
        fn drop(&mut self) {
            self.db.pending.lock().blobs.push(self.blob);
            self.db.flush_cleanup();
        }
    }

    struct BlobGuard<'a> {
        inner: Option<PyMappedMutexGuard<'a, BlobInner>>,
        _db: SqliteOperation,
    }

    impl<'a> BlobGuard<'a> {
        fn new(inner: PyMappedMutexGuard<'a, BlobInner>) -> Self {
            let db = SqliteOperation(inner.db.clone());
            Self {
                inner: Some(inner),
                _db: db,
            }
        }
    }

    impl Deref for BlobGuard<'_> {
        type Target = BlobInner;

        fn deref(&self) -> &Self::Target {
            self.inner.as_deref().unwrap()
        }
    }

    impl DerefMut for BlobGuard<'_> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            self.inner.as_deref_mut().unwrap()
        }
    }

    impl Drop for BlobGuard<'_> {
        fn drop(&mut self) {
            // The operation drains pending cleanup after releasing the blob lock.
            self.inner.take();
        }
    }

    #[cfg(test)]
    #[cfg(feature = "threading")]
    mod tests {
        use super::*;
        use core::time::Duration;
        use std::sync::mpsc;

        #[test]
        fn blob_operation_drains_contended_cleanup() {
            let mut raw = null_mut();
            assert_eq!(
                unsafe {
                    sqlite3_open_v2(
                        c":memory:".as_ptr(),
                        &mut raw,
                        SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX,
                        null(),
                    )
                },
                SQLITE_OK
            );
            let db = PyRc::new(Sqlite::from(SqliteRaw { db: raw }));
            assert!(
                db.exec(c"CREATE TABLE t(b); INSERT INTO t VALUES (zeroblob(1))")
                    .is_ok()
            );
            let open_blob = || {
                let mut blob = null_mut();
                assert_eq!(
                    unsafe {
                        sqlite3_blob_open(
                            db.db,
                            c"main".as_ptr(),
                            c"t".as_ptr(),
                            c"b".as_ptr(),
                            1,
                            0,
                            &mut blob,
                        )
                    },
                    SQLITE_OK
                );
                BlobInner {
                    blob: SqliteBlob { blob },
                    offset: 0,
                    db: db.clone(),
                }
            };
            let inner = PyMutex::new(Some(open_blob()));
            let retired = open_blob();
            let guard = BlobGuard::new(PyMutexGuard::map(inner.lock(), |x| x.as_mut().unwrap()));
            let (locked_tx, locked_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let other = db.clone();
            let worker = std::thread::spawn(move || {
                let _lock = other.raw.lock();
                locked_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            });
            locked_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            // Reproduce close() losing both nonblocking cleanup attempts.
            db.pending.lock().clear_callbacks = true;
            db.flush_cleanup();
            drop(retired);
            assert_eq!(db.pending.lock().blobs.len(), 1);
            release_tx.send(()).unwrap();
            worker.join().unwrap();
            assert!(db.pending.lock().clear_callbacks);
            drop(guard);
            assert!(db.pending.lock().is_empty());
        }
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION), with(AsMapping, AsNumber, AsSequence))]
    impl Blob {
        #[pymethod]
        fn close(zelf: &Py<Self>) {
            with_deferred_drops(|| {
                let inner = zelf.inner.lock().take();
                drop(inner);
            });
        }

        fn ensure_connection_open(&self, vm: &VirtualMachine) -> PyResult<()> {
            if self.connection.is_closed() {
                Err(new_programming_error(
                    vm,
                    "Cannot operate on a closed database".to_owned(),
                ))
            } else {
                Ok(())
            }
        }

        #[pymethod]
        fn read(
            zelf: &Py<Self>,
            length: OptionalArg<c_int>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<PyBytes>> {
            zelf.ensure_connection_open(vm)?;

            let mut length = length.unwrap_or(-1);
            let mut inner = zelf.inner(vm)?;
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
                inner.check(ret, vm)?;
                unsafe { buf.set_len(length as usize) };
                inner.offset += length;
                Ok(vm.ctx.new_bytes(buf))
            }
        }

        #[pymethod]
        fn write(zelf: &Py<Self>, data: PyBuffer, vm: &VirtualMachine) -> PyResult<()> {
            zelf.ensure_connection_open(vm)?;
            let mut inner = zelf.inner(vm)?;
            let blob_len = inner.blob.bytes();
            let length = Self::expect_write(blob_len, data.desc.len, inner.offset, vm)?;

            let ret = data.contiguous_or_collect(|buf| {
                inner.blob.write(buf.as_ptr().cast(), length, inner.offset)
            });

            inner.check(ret, vm)?;
            inner.offset += length;
            Ok(())
        }

        #[pymethod]
        fn tell(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<c_int> {
            zelf.ensure_connection_open(vm)?;
            zelf.inner(vm).map(|x| x.offset)
        }

        #[pymethod]
        fn seek(
            zelf: &Py<Self>,
            mut offset: c_int,
            origin: OptionalArg<c_int>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            zelf.ensure_connection_open(vm)?;
            let origin = origin.unwrap_or(libc::SEEK_SET);
            let mut inner = zelf.inner(vm)?;
            let blob_len = inner.blob.bytes();

            let overflow_err = || vm.new_overflow_error("seek offset results in overflow");

            match origin {
                libc::SEEK_SET => {}
                libc::SEEK_CUR => {
                    offset = offset.checked_add(inner.offset).ok_or_else(overflow_err)?
                }
                libc::SEEK_END => offset = offset.checked_add(blob_len).ok_or_else(overflow_err)?,
                _ => {
                    return Err(vm.new_value_error(
                        "'origin' should be os.SEEK_SET, os.SEEK_CUR, or os.SEEK_END",
                    ));
                }
            }

            if offset < 0 || offset > blob_len {
                Err(vm.new_value_error("offset out of blob range"))
            } else {
                inner.offset = offset;
                Ok(())
            }
        }

        #[pymethod]
        fn __enter__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            zelf.ensure_connection_open(vm)?;
            let _ = zelf.inner(vm)?;
            Ok(zelf)
        }

        #[pymethod]
        fn __exit__(zelf: &Py<Self>, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            zelf.ensure_connection_open(vm)?;
            let _ = zelf.inner(vm)?;
            Self::close(zelf);
            Ok(())
        }

        fn inner(&self, vm: &VirtualMachine) -> PyResult<BlobGuard<'_>> {
            let guard = self.inner.lock();
            if guard.is_some() {
                Ok(BlobGuard::new(PyMutexGuard::map(guard, |x| {
                    x.as_mut().unwrap()
                })))
            } else {
                Err(new_programming_error(
                    vm,
                    "Cannot operate on a closed blob.".to_owned(),
                ))
            }
        }

        fn wrapped_index(
            index: PyIntRef,
            object: &PyObject,
            length: c_int,
            vm: &VirtualMachine,
        ) -> PyResult<c_int> {
            let mut index = index.try_to_primitive::<isize>(vm).map_err(|_| {
                let class = object.class();
                let type_name = class.slot_name();
                let mut end = type_name.len().min(200);
                while !type_name.is_char_boundary(end) {
                    end -= 1;
                }
                vm.new_index_error(format!(
                    "cannot fit '{}' into an index-sized integer",
                    &type_name[..end]
                ))
            })?;
            if index < 0 {
                index += length as isize;
            }
            if index < 0 || index >= length as isize {
                Err(vm.new_index_error("Blob index out of range"))
            } else {
                Ok(index as c_int)
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
                Err(vm.new_value_error("data longer than blob length"))
            }
        }

        fn subscript(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
            with_deferred_drops(|| Self::subscript_inner(self, needle, vm))
        }

        fn subscript_inner(&self, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
            self.ensure_connection_open(vm)?;
            let inner = self.inner(vm)?;
            if let Some(index) = needle.try_index_opt(vm) {
                let blob_len = inner.blob.bytes();
                let index = Self::wrapped_index(index?, needle, blob_len, vm)?;
                let mut byte: u8 = 0;
                let ret = inner.blob.read_single(&mut byte, index);
                inner.check(ret, vm).map(|_| vm.ctx.new_int(byte).into())
            } else if let Some(slice) = needle.downcast_ref::<PySlice>() {
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
                    inner.check(ret, vm)?;
                    unsafe { buf.set_len(length) };
                } else {
                    let iter = SaturatedSliceIter::from_adjust_indices(range, step, length);
                    let mut byte: u8 = 0;
                    for index in iter {
                        let ret = inner.blob.read_single(&mut byte, index as c_int);
                        inner.check(ret, vm)?;
                        buf.push(byte);
                    }
                }
                Ok(vm.ctx.new_bytes(buf).into())
            } else {
                Err(vm.new_type_error("Blob indices must be integers"))
            }
        }

        fn ass_subscript(
            &self,
            needle: &PyObject,
            value: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            with_deferred_drops(|| Self::ass_subscript_inner(self, needle, value, vm))
        }

        fn ass_subscript_inner(
            &self,
            needle: &PyObject,
            value: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            self.ensure_connection_open(vm)?;
            let inner = self.inner(vm)?;

            if let Some(index) = needle.try_index_opt(vm) {
                // Handle single item assignment: blob[i] = b
                let Some(value) = value else {
                    return Err(vm.new_type_error("Blob doesn't support item deletion"));
                };
                let Some(int_val) = value.downcast_ref::<PyInt>() else {
                    return Err(vm.new_type_error(format!(
                        "'{}' object cannot be interpreted as an integer",
                        value.class()
                    )));
                };
                let blob_len = inner.blob.bytes();
                let index = Self::wrapped_index(index?, needle, blob_len, vm)?;
                // Mirror CPython ass_subscript_index: use PyLong_AsLong, treat any
                // overflow (e.g. 2**65) as -1, then validate the [0, 255] range.
                let val = int_val.as_bigint().to_i64().unwrap_or(-1);
                if !(0..=255).contains(&val) {
                    return Err(vm.new_value_error("byte must be in range(0, 256)"));
                }
                let ret = inner.blob.write_single(val as u8, index);
                inner.check(ret, vm)
            } else if let Some(slice) = needle.downcast_ref::<PySlice>() {
                // Handle slice assignment: blob[a:b:c] = b"..."
                let Some(value) = value else {
                    return Err(vm.new_type_error("Blob doesn't support slice deletion"));
                };
                let value_buf = PyBuffer::try_from_borrowed_object(vm, &value)?;

                let buf = value_buf
                    .as_contiguous()
                    .ok_or_else(|| vm.new_buffer_error("underlying buffer is not C-contiguous"))?;

                let blob_len = inner.blob.bytes();
                let slice = slice.to_saturated(vm)?;
                let (range, step, slice_len) = slice.adjust_indices(blob_len as usize);

                if step == 0 {
                    return Err(vm.new_value_error("slice step cannot be zero"));
                }

                if buf.len() != slice_len {
                    return Err(vm.new_index_error("Blob slice assignment is wrong size"));
                }

                if slice_len == 0 {
                    return Ok(());
                }

                if step == 1 {
                    let ret = inner.blob.write(
                        buf.as_ptr().cast(),
                        buf.len() as c_int,
                        range.start as c_int,
                    );
                    inner.check(ret, vm)
                } else {
                    let span_len = range.end - range.start;
                    let range_start = range.start;
                    let mut temp_buf = vec![0u8; span_len];

                    let ret = inner.blob.read(
                        temp_buf.as_mut_ptr().cast(),
                        span_len as c_int,
                        range_start as c_int,
                    );
                    inner.check(ret, vm)?;

                    let iter = SaturatedSliceIter::from_adjust_indices(range, step, slice_len);
                    for (i_in_src, abs_idx) in iter.enumerate() {
                        temp_buf[abs_idx - range_start] = buf[i_in_src];
                    }

                    let ret = inner.blob.write(
                        temp_buf.as_ptr().cast(),
                        span_len as c_int,
                        range_start as c_int,
                    );
                    inner.check(ret, vm)
                }
            } else {
                Err(vm.new_type_error("Blob indices must be integers"))
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

    impl AsNumber for Blob {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                add: Some(|a, b, vm| {
                    Err(vm.new_type_error(format!(
                        "unsupported operand type(s) for +: '{}' and '{}'",
                        a.class().name(),
                        b.class().name()
                    )))
                }),
                multiply: Some(|a, b, vm| {
                    Err(vm.new_type_error(format!(
                        "unsupported operand type(s) for *: '{}' and '{}'",
                        a.class().name(),
                        b.class().name()
                    )))
                }),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    impl AsSequence for Blob {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                length: None,
                concat: None,
                repeat: None,
                item: None,
                ass_item: None,
                contains: atomic_func!(|seq, _needle, vm| {
                    Err(vm.new_type_error(format!(
                        "argument of type '{}' is not iterable",
                        seq.obj.class().name(),
                    )))
                }),
                inplace_concat: None,
                inplace_repeat: None,
            };
            &AS_SEQUENCE
        }
    }

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug, PyPayload)]
    struct PrepareProtocol {}

    #[pyclass()]
    impl PrepareProtocol {}

    #[pyattr]
    #[pyclass(module = "sqlite3", name = "Statement")]
    #[derive(PyPayload)]
    struct Statement {
        st: PyMutex<SqliteStatement>,
        pub is_dml: bool,
        in_use: PyAtomic<bool>,
    }

    impl Debug for Statement {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> Result<(), core::fmt::Error> {
            write!(
                f,
                "{} Statement",
                if self.is_dml { "DML" } else { "Non-DML" }
            )
        }
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl Statement {
        fn new(
            connection: &Connection,
            sql: PyUtf8StrRef,
            vm: &VirtualMachine,
        ) -> PyResult<Option<Self>> {
            let sql_cstr = sql.to_cstring(vm).map_err(|_| {
                new_programming_error(vm, "statement contains a null character.".to_owned())
            })?;

            let db = connection.db_lock(vm)?;
            db.sql_limit(sql.byte_len(), vm)?;
            let mut tail = null();
            let st = db.prepare(&sql_cstr, &mut tail, vm);
            connection.check_open(vm)?;

            let Some(st) = st? else {
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
                in_use: Radium::new(false),
            }))
        }

        fn lock(&self) -> PyMutexGuard<'_, SqliteStatement> {
            self.st.lock()
        }
    }

    struct StatementCache {
        capacity: usize,
        generation: u64,
        entries: indexmap::IndexMap<Box<str>, PyRef<Statement>>,
    }

    impl StatementCache {
        fn new(capacity: c_int) -> Self {
            Self {
                capacity: capacity.max(0) as usize,
                generation: 0,
                entries: indexmap::IndexMap::new(),
            }
        }
    }

    #[derive(Debug)]
    struct StatementLease(PyRef<Statement>);

    impl StatementLease {
        fn acquire(statement: PyRef<Statement>) -> Option<Self> {
            statement
                .in_use
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .ok()
                .map(|_| Self(statement))
        }

        fn database(
            &self,
            connection: &Connection,
            vm: &VirtualMachine,
        ) -> PyResult<SqliteOperation> {
            connection.check_open(vm)?;
            // The statement owns a live connection, not just memory retained by
            // sqlite3_close_v2. Reinitialization may replace Connection.db while
            // existing cursors continue reading this original database.
            Ok(SqliteOperation(self.lock().db.clone()))
        }
    }

    impl Deref for StatementLease {
        type Target = Statement;

        fn deref(&self) -> &Statement {
            &self.0
        }
    }

    // SAFETY: the lease owns exactly one reference to its statement.
    unsafe impl Traverse for StatementLease {
        fn traverse(&self, tracer: &mut TraverseFn<'_>) {
            self.0.traverse(tracer);
        }
    }

    impl Drop for StatementLease {
        fn drop(&mut self) {
            with_deferred_drops(|| {
                let statement = self.0.lock();
                let db = SqliteOperation(statement.db.clone());
                statement.reset();
                // Cached statements must not retain previous parameter buffers.
                unsafe { sqlite3_clear_bindings(statement.st) };
                drop(statement);
                self.0.in_use.store(false, Ordering::Release);
                // Reset can delay cleanup queued by another thread's BLOB close.
                drop(db);
            });
        }
    }

    struct Sqlite {
        raw: SqliteRaw,
        pending: PyMutex<SqliteCleanup>,
        callbacks: PyMutex<CallbackSlots>,
    }

    #[derive(Default)]
    struct CallbackSlots {
        trace: Option<PyRc<CallbackData>>,
        progress: Option<PyRc<CallbackData>>,
        authorizer: Option<PyRc<CallbackData>>,
        closed: bool,
    }

    #[derive(Clone, Copy)]
    enum CallbackKind {
        Trace,
        Progress(c_int),
        Authorizer,
    }

    impl CallbackKind {
        fn slot(self, slots: &mut CallbackSlots) -> &mut Option<PyRc<CallbackData>> {
            match self {
                Self::Trace => &mut slots.trace,
                Self::Progress(_) => &mut slots.progress,
                Self::Authorizer => &mut slots.authorizer,
            }
        }

        // The caller holds the native mutex and retains data until unregistration.
        fn register(self, db: SqliteRaw, data: *mut CallbackData) -> c_int {
            unsafe {
                match self {
                    Self::Trace => sqlite3_trace_v2(
                        db.db,
                        SQLITE_TRACE_STMT,
                        (!data.is_null()).then_some(CallbackData::trace_callback),
                        data.cast(),
                    ),
                    Self::Progress(n) => {
                        sqlite3_progress_handler(
                            db.db,
                            n,
                            (!data.is_null()).then_some(CallbackData::progress_callback),
                            data.cast(),
                        );
                        SQLITE_OK
                    }
                    Self::Authorizer => sqlite3_set_authorizer(
                        db.db,
                        (!data.is_null()).then_some(CallbackData::authorizer_callback),
                        data.cast(),
                    ),
                }
            }
        }
    }

    #[derive(Default)]
    struct SqliteCleanup {
        statements: Vec<SqliteStatementRaw>,
        blobs: Vec<SqliteBlob>,
        rollback: bool,
        clear_callbacks: bool,
    }

    impl SqliteCleanup {
        fn is_empty(&self) -> bool {
            self.statements.is_empty()
                && self.blobs.is_empty()
                && !self.rollback
                && !self.clear_callbacks
        }
    }

    impl From<SqliteRaw> for Sqlite {
        fn from(raw: SqliteRaw) -> Self {
            Self {
                raw,
                pending: PyMutex::new(SqliteCleanup::default()),
                callbacks: PyMutex::new(CallbackSlots::default()),
            }
        }
    }

    impl Sqlite {
        fn flush_cleanup(&self) {
            self.drain_cleanup(false);
        }

        fn drain_cleanup(&self, wait: bool) {
            if self.pending.lock().is_empty() {
                return;
            }
            with_deferred_drops(|| {
                loop {
                    let guard = if wait {
                        Some(self.raw.lock())
                    } else {
                        self.raw.try_lock()
                    };
                    let Some(guard) = guard else {
                        return;
                    };
                    // Never keep a Rust mutex while SQLite can invoke Python.
                    let pending = core::mem::take(&mut *self.pending.lock());
                    if pending.rollback && !self.is_autocommit() {
                        // close() has already returned; like sqlite3_close_v2,
                        // deferred rollback cannot report an error to its caller.
                        unsafe {
                            sqlite3_exec(
                                self.db,
                                c"ROLLBACK".as_ptr(),
                                None,
                                null_mut(),
                                null_mut(),
                            )
                        };
                    }
                    for statement in pending.statements {
                        unsafe { sqlite3_finalize(statement.st) };
                    }
                    for blob in pending.blobs {
                        unsafe { sqlite3_blob_close(blob.blob) };
                    }
                    let callbacks = pending.clear_callbacks.then(|| self.clear_callbacks());
                    drop(guard);
                    drop(callbacks);
                    // An enqueue after this check can acquire the native mutex and
                    // drain itself; one made while it was locked must be retried.
                    if self.pending.lock().is_empty() {
                        return;
                    }
                }
            });
        }

        fn close_callbacks(&self, rollback: bool, vm: &VirtualMachine) -> PyResult<()> {
            self.callbacks.lock().closed = true;
            if let Some(guard) = self.raw.try_lock() {
                let result = if rollback && !self.is_autocommit() {
                    self._exec(c"ROLLBACK", vm)
                } else {
                    Ok(())
                };
                self.pending.lock().clear_callbacks = true;
                drop(guard);
                self.flush_cleanup();
                result
            } else {
                {
                    let mut pending = self.pending.lock();
                    pending.rollback |= rollback;
                    pending.clear_callbacks = true;
                }
                // Retry after enqueueing, in case the active call just ended.
                self.flush_cleanup();
                Ok(())
            }
        }

        // The native mutex is held. SQLite must stop using the pointers first.
        fn clear_callbacks(&self) -> [Option<PyRc<CallbackData>>; 3] {
            for kind in [
                CallbackKind::Trace,
                CallbackKind::Progress(0),
                CallbackKind::Authorizer,
            ] {
                kind.register(self.raw, null_mut());
            }
            let mut slots = self.callbacks.lock();
            [
                slots.trace.take(),
                slots.progress.take(),
                slots.authorizer.take(),
            ]
        }
    }

    // An operation retains its database and drains work deferred by a close
    // callback after SQLite has released its native connection mutex.
    struct SqliteOperation(PyRc<Sqlite>);

    impl Deref for SqliteOperation {
        type Target = Sqlite;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl Drop for SqliteOperation {
        fn drop(&mut self) {
            self.flush_cleanup();
        }
    }

    impl SqliteOperation {
        fn prepare(
            &self,
            sql: &CStr,
            tail: &mut *const libc::c_char,
            vm: &VirtualMachine,
        ) -> PyResult<Option<SqliteStatement>> {
            self.raw.prepare(sql, tail, vm).map(|statement| {
                statement.map(|raw| SqliteStatement {
                    raw,
                    db: self.0.clone(),
                })
            })
        }
    }

    struct SqliteMutex(*mut libsqlite3_sys::sqlite3_mutex);

    impl Drop for SqliteMutex {
        fn drop(&mut self) {
            unsafe { sqlite3_mutex_leave(self.0) };
        }
    }

    struct SqliteError {
        code: c_int,
        extended: c_int,
        message: String,
    }

    impl SqliteError {
        fn into_exception(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
            let typ = exception_type_from_errcode(self.code, vm);
            raise_exception(typ.to_owned(), self.extended, self.message, vm)
        }
    }

    // sqlite3_close_v2 is not exported by libsqlite3-sys, so we declare it manually.
    // It handles "zombie close" - if there are still unfinalized statements,
    // the database will be closed when the last statement is finalized.
    unsafe extern "C" {
        fn sqlite3_close_v2(db: *mut sqlite3) -> c_int;
    }

    impl Drop for Sqlite {
        fn drop(&mut self) {
            // No operation or statement owner remains. Drain native handles
            // queued by their destructors before closing the database.
            self.drain_cleanup(true);
            // Use sqlite3_close_v2 for safe closing even with active statements
            with_deferred_drops(|| {
                let callbacks = {
                    let _guard = self.raw.lock();
                    self.clear_callbacks()
                };
                drop(callbacks);
                unsafe { sqlite3_close_v2(self.raw.db) };
            });
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

    cfg_select! {
        feature = "threading" => {
            unsafe impl Send for SqliteStatement {}
            // unsafe impl Sync for SqliteStatement {}
            unsafe impl Send for Sqlite {}
            // SAFETY: connections use FULLMUTEX and require a native mutex.
            // Owned references delay closing until active operations finish.
            unsafe impl Sync for Sqlite {}
            unsafe impl Send for SqliteBlob {}
        }
        _ => {}
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
        fn lock(self) -> SqliteMutex {
            let mutex = unsafe { sqlite3_db_mutex(self.db) };
            unsafe { sqlite3_mutex_enter(mutex) };
            SqliteMutex(mutex)
        }

        fn try_lock(self) -> Option<SqliteMutex> {
            let mutex = unsafe { sqlite3_db_mutex(self.db) };
            if unsafe { sqlite3_mutex_try(mutex) } == SQLITE_OK {
                Some(SqliteMutex(mutex))
            } else {
                None
            }
        }

        fn call(self, call: impl FnOnce() -> c_int) -> Result<c_int, SqliteError> {
            let _guard = self.lock();
            let ret = call();
            if matches!(ret, SQLITE_OK | SQLITE_ROW | SQLITE_DONE) {
                Ok(ret)
            } else {
                Err(self.error_snapshot())
            }
        }

        fn check(self, result: Result<c_int, SqliteError>, vm: &VirtualMachine) -> PyResult<()> {
            result.map(drop).map_err(|err| err.into_exception(vm))
        }

        fn checked(self, call: impl FnOnce() -> c_int, vm: &VirtualMachine) -> PyResult<()> {
            self.check(self.call(call), vm)
        }

        // The caller holds the connection mutex from before the failed call.
        fn error_snapshot(self) -> SqliteError {
            unsafe {
                SqliteError {
                    code: sqlite3_errcode(self.db),
                    extended: sqlite3_extended_errcode(self.db),
                    message: CStr::from_ptr(sqlite3_errmsg(self.db))
                        .to_string_lossy()
                        .into_owned(),
                }
            }
        }

        fn error_extended(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
            let error = {
                let _guard = self.lock();
                self.error_snapshot()
            };
            error.into_exception(vm)
        }

        fn open(path: *const libc::c_char, uri: bool, vm: &VirtualMachine) -> PyResult<Self> {
            let mut db = null_mut();
            let ret = unsafe {
                sqlite3_open_v2(
                    path,
                    &raw mut db,
                    SQLITE_OPEN_READWRITE
                        | SQLITE_OPEN_CREATE
                        | SQLITE_OPEN_FULLMUTEX
                        | if uri { SQLITE_OPEN_URI } else { 0 },
                    null(),
                )
            };
            let zelf = Self { db };
            if ret == SQLITE_OK {
                Ok(zelf)
            } else {
                Err(zelf.error_extended(vm))
            }
        }

        fn _exec(self, sql: &CStr, vm: &VirtualMachine) -> PyResult<()> {
            self.exec(sql).map_err(|err| err.into_exception(vm))
        }

        fn exec(self, sql: &CStr) -> Result<(), SqliteError> {
            self.call(|| unsafe {
                sqlite3_exec(self.db, sql.as_ptr(), None, null_mut(), null_mut())
            })
            .map(drop)
        }

        fn prepare(
            self,
            sql: &CStr,
            tail: &mut *const libc::c_char,
            vm: &VirtualMachine,
        ) -> PyResult<Option<SqliteStatementRaw>> {
            let mut st = null_mut();
            self.checked(
                || unsafe { sqlite3_prepare_v2(self.db, sql.as_ptr(), -1, &mut st, tail) },
                vm,
            )?;
            if st.is_null() {
                Ok(None)
            } else {
                Ok(Some(SqliteStatementRaw::from(st)))
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

        // The caller holds the native mutex across checking the mode and the
        // complete transition, including reopening a PEP 249 transaction.
        fn end_transaction(self, sql: &CStr, mode: AutocommitMode) -> Result<(), SqliteError> {
            match mode {
                AutocommitMode::Legacy if !self.is_autocommit() => self.exec(sql),
                AutocommitMode::Disabled => {
                    self.exec(sql)?;
                    self.exec(c"BEGIN")
                }
                _ => Ok(()),
            }
        }

        fn begin_transaction(self, isolation_level: Option<PyStrRef>) -> Result<(), SqliteError> {
            let Some(isolation_level) = isolation_level else {
                return Ok(());
            };
            let mut s = Vec::with_capacity(16);
            s.extend(b"BEGIN ");
            s.extend(isolation_level.expect_str().bytes());
            s.push(b'\0');
            // SAFETY: isolation_level is checked for interior NULs on construction
            let cs = unsafe { CStr::from_bytes_with_nul_unchecked(&s) };
            self.exec(cs)
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
            name: &CStr,
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
            let ret = self.call(|| unsafe {
                sqlite3_create_function_v2(
                    self.db,
                    name.as_ptr(),
                    narg,
                    flags,
                    data,
                    func,
                    step,
                    finalize,
                    destroy,
                )
            });
            self.check(ret, vm)
                .map_err(|_| new_operational_error(vm, "Error creating function".to_owned()))
        }
    }

    struct SqliteStatement {
        raw: SqliteStatementRaw,
        // Keep the original database usable for cursors surviving __init__.
        db: PyRc<Sqlite>,
    }

    impl Drop for SqliteStatement {
        fn drop(&mut self) {
            // Finalization may wait for a callback running on another thread.
            // Queue only the raw handle, so pending cleanup cannot form an
            // ownership cycle with the database.
            self.db.pending.lock().statements.push(self.raw);
            self.db.flush_cleanup();
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

    struct SqliteStep {
        has_row: bool,
        changes: c_int,
        lastrowid: i64,
    }

    impl From<*mut sqlite3_stmt> for SqliteStatementRaw {
        fn from(st: *mut sqlite3_stmt) -> Self {
            Self { st }
        }
    }

    impl SqliteStatementRaw {
        fn step(self) -> Result<SqliteStep, SqliteError> {
            let db = SqliteRaw::from(self);
            let _guard = db.lock();
            let ret = db.call(|| unsafe { sqlite3_step(self.st) })?;
            // These values belong to the connection, not the statement. Another
            // cursor must not overwrite them between step and the snapshot.
            Ok(SqliteStep {
                has_row: ret == SQLITE_ROW,
                changes: db.changes(),
                lastrowid: db.lastrowid(),
            })
        }

        fn step_checked(self, vm: &VirtualMachine) -> PyResult<SqliteStep> {
            let ret = self.step();

            if let Some(exc) = unsafe { user_function_exception().swap(None) } {
                Err(exc)
            } else {
                ret.map_err(|err| err.into_exception(vm))
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
                    PrepareProtocol::class(&vm.ctx).to_owned(),
                    |x| Ok(x.to_owned()),
                    vm,
                )?;
                &adapted
            } else {
                parameter
            };

            let db = SqliteRaw::from(self);
            let ret = if vm.is_none(obj) {
                db.call(|| unsafe { sqlite3_bind_null(self.st, pos) })
            } else if let Some(val) = obj.downcast_ref::<PyInt>() {
                let val = val.try_to_primitive::<i64>(vm).map_err(|_| {
                    vm.new_overflow_error("Python int too large to convert to SQLite INTEGER")
                })?;
                db.call(|| unsafe { sqlite3_bind_int64(self.st, pos, val) })
            } else if let Some(val) = obj.downcast_ref::<PyFloat>() {
                let val = val.to_f64();
                db.call(|| unsafe { sqlite3_bind_double(self.st, pos, val) })
            } else if let Some(val) = obj.downcast_ref::<PyStr>() {
                let val = val.try_as_utf8(vm)?;
                let (ptr, len) = str_to_ptr_len(val, vm)?;
                db.call(|| unsafe { sqlite3_bind_text(self.st, pos, ptr, len, SQLITE_TRANSIENT()) })
            } else if let Ok(buffer) = PyBuffer::try_from_borrowed_object(vm, obj) {
                let (ptr, len) = buffer_to_ptr_len(&buffer, vm)?;
                db.call(|| unsafe { sqlite3_bind_blob(self.st, pos, ptr, len, SQLITE_TRANSIENT()) })
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

            db.check(ret, vm)
        }

        fn bind_parameters(self, parameters: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            if let Some(dict) = parameters.downcast_ref::<PyDict>() {
                self.bind_parameters_name(dict, vm)
            } else if let Ok(seq) = parameters.try_sequence(vm) {
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

                let val = match dict.get_item_opt(name, vm)? {
                    Some(val) => val,
                    None => {
                        return Err(new_programming_error(
                            vm,
                            format!("You did not supply a value for binding parameter :{name}.",),
                        ));
                    }
                };

                self.bind_parameter(i, &val, vm)?;
            }
            Ok(())
        }

        fn bind_parameter_count(self) -> c_int {
            unsafe { sqlite3_bind_parameter_count(self.st) }
        }

        fn bind_parameters_sequence(
            self,
            seq: PySequence<'_>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let num_needed = self.bind_parameter_count();
            let num_supplied = seq.length(vm)?;

            if num_supplied != num_needed as usize {
                return Err(new_programming_error(
                    vm,
                    format!(
                        "Incorrect number of bindings supplied. The current statement uses {num_needed}, and {num_supplied} were supplied."
                    ),
                ));
            }

            for i in 1..=num_needed {
                let name = unsafe { sqlite3_bind_parameter_name(self.st, i) };
                if !name.is_null() && unsafe { *name } != b'?' as libc::c_char {
                    let name_str = ptr_to_str(name, vm)?;
                    return Err(new_programming_error(
                        vm,
                        format!(
                            "Binding {i} ('{name_str}') is a named parameter, but you supplied a sequence which requires nameless (qmark) placeholders."
                        ),
                    ));
                }
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

        fn columns_name(self, detect_types: i32, vm: &VirtualMachine) -> PyResult<Vec<PyStrRef>> {
            let count = self.column_count();
            (0..count)
                .map(|i| {
                    let name = self.column_name(i);
                    let name_str = ptr_to_str(name, vm)?;

                    // If PARSE_COLNAMES is enabled, strip everything after the first '[' (and preceding space)
                    let processed_name = if detect_types & PARSE_COLNAMES != 0
                        && let Some(bracket_pos) = name_str.find('[')
                    {
                        // Check if there's a single space before '[' and remove it (CPython compatibility)
                        let end_pos = if bracket_pos > 0
                            && name_str.chars().nth(bracket_pos - 1) == Some(' ')
                        {
                            bracket_pos - 1
                        } else {
                            bracket_pos
                        };

                        &name_str[..end_pos]
                    } else {
                        name_str
                    };

                    Ok(vm.ctx.new_str(processed_name))
                })
                .collect()
        }

        fn columns_description(
            self,
            detect_types: i32,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyTupleRef>> {
            if self.column_count() == 0 {
                return Ok(None);
            }
            let columns = self
                .columns_name(detect_types, vm)?
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

        fn aggregate_context(self, create: bool) -> *mut *const PyObject {
            let size = if create {
                core::mem::size_of::<*const PyObject>() as c_int
            } else {
                0
            };
            unsafe { sqlite3_aggregate_context(self.ctx, size).cast() }
        }

        fn result_exception(self, vm: &VirtualMachine, exc: PyBaseExceptionRef, msg: &CStr) {
            if exc.fast_isinstance(vm.ctx.exceptions.memory_error) {
                unsafe { sqlite3_result_error_nomem(self.ctx) }
            } else if exc.fast_isinstance(vm.ctx.exceptions.overflow_error) {
                unsafe { sqlite3_result_error_toobig(self.ctx) }
            } else {
                unsafe { sqlite3_result_error(self.ctx, msg.as_ptr(), -1) }
            }
            if enable_traceback().load(Ordering::Relaxed) {
                vm.print_exception(&exc);
            }
        }

        fn db_handle(self) -> *mut sqlite3 {
            unsafe { sqlite3_context_db_handle(self.ctx) }
        }

        fn result_from_object(self, val: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            unsafe {
                if vm.is_none(val) {
                    sqlite3_result_null(self.ctx)
                } else if let Some(val) = val.downcast_ref::<PyInt>() {
                    sqlite3_result_int64(self.ctx, val.try_to_primitive(vm)?)
                } else if let Some(val) = val.downcast_ref::<PyFloat>() {
                    sqlite3_result_double(self.ctx, val.to_f64())
                } else if let Some(val) = val.downcast_ref::<PyStr>() {
                    let val = val.try_as_utf8(vm)?;
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
                    let text = String::from_utf8(text)
                        .map_err(|_| vm.new_value_error("invalid utf-8 with SQLITE_TEXT"))?;
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

    fn check_num_params(
        db: &Sqlite,
        n: c_int,
        param_name: &str,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let limit = unsafe { sqlite3_limit(db.db, SQLITE_LIMIT_FUNCTION_ARG, -1) };
        if n < -1 || n > limit {
            return Err(new_programming_error(
                vm,
                format!("'{param_name}' must be between -1 and {limit}, not {n}"),
            ));
        }
        Ok(())
    }

    fn is_int_dbconfig(op: c_int) -> bool {
        use libsqlite3_sys::*;
        matches!(
            op,
            SQLITE_DBCONFIG_ENABLE_FKEY
                | SQLITE_DBCONFIG_ENABLE_TRIGGER
                | SQLITE_DBCONFIG_ENABLE_FTS3_TOKENIZER
                | SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION
                | SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE
                | SQLITE_DBCONFIG_ENABLE_QPSG
                | SQLITE_DBCONFIG_TRIGGER_EQP
                | SQLITE_DBCONFIG_RESET_DATABASE
                | SQLITE_DBCONFIG_DEFENSIVE
                | SQLITE_DBCONFIG_WRITABLE_SCHEMA
                | SQLITE_DBCONFIG_LEGACY_ALTER_TABLE
                | SQLITE_DBCONFIG_DQS_DDL
                | SQLITE_DBCONFIG_DQS_DML
                | SQLITE_DBCONFIG_ENABLE_VIEW
                | SQLITE_DBCONFIG_LEGACY_FILE_FORMAT
                | SQLITE_DBCONFIG_TRUSTED_SCHEMA
                | SQLITE_DBCONFIG_STMT_SCANSTATUS
                | SQLITE_DBCONFIG_REVERSE_SCANORDER
                | SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE
                | SQLITE_DBCONFIG_ENABLE_ATTACH_WRITE
                | SQLITE_DBCONFIG_ENABLE_COMMENTS
        )
    }

    fn ptr_to_str<'a>(p: *const libc::c_char, vm: &VirtualMachine) -> PyResult<&'a str> {
        if p.is_null() {
            return Err(vm.new_memory_error("string pointer is null"));
        }
        unsafe { CStr::from_ptr(p).to_str() }
            .map_err(|_| vm.new_value_error("Invalid UTF-8 codepoint"))
    }

    fn ptr_to_str_or_none(p: *const libc::c_char, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        if p.is_null() {
            return Ok(vm.ctx.none());
        }
        let s = unsafe { CStr::from_ptr(p) }
            .to_str()
            .map_err(|_| vm.new_value_error("Invalid UTF-8 codepoint".to_owned()))?;
        Ok(vm.ctx.new_str(s).into())
    }

    fn ptr_to_string(
        p: *const u8,
        nbytes: c_int,
        db: *mut sqlite3,
        vm: &VirtualMachine,
    ) -> PyResult<Box<str>> {
        let s = ptr_to_vec(p, nbytes, db, vm)?;
        String::from_utf8(s)
            .map(Into::into)
            .map_err(|_| vm.new_value_error("invalid utf-8"))
    }

    fn ptr_to_vec(
        p: *const u8,
        nbytes: c_int,
        db: *mut sqlite3,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        if p.is_null() {
            if !db.is_null() && unsafe { sqlite3_errcode(db) } == SQLITE_NOMEM {
                Err(vm.new_memory_error("sqlite out of memory"))
            } else {
                Ok(vec![])
            }
        } else if nbytes < 0 {
            Err(vm.new_system_error("negative size with ptr"))
        } else {
            Ok(unsafe { core::slice::from_raw_parts(p.cast(), nbytes as usize) }.to_vec())
        }
    }

    fn str_to_ptr_len(s: &PyUtf8Str, vm: &VirtualMachine) -> PyResult<(*const libc::c_char, i32)> {
        let s_str = s.as_str();
        let len = c_int::try_from(s_str.len())
            .map_err(|_| vm.new_overflow_error("TEXT longer than INT_MAX bytes"))?;
        let ptr = s_str.as_ptr().cast();
        Ok((ptr, len))
    }

    fn buffer_to_ptr_len(buffer: &PyBuffer, vm: &VirtualMachine) -> PyResult<(*const c_void, i32)> {
        let bytes = buffer
            .as_contiguous()
            .ok_or_else(|| vm.new_buffer_error("underlying buffer is not C-contiguous"))?;
        let len = c_int::try_from(bytes.len())
            .map_err(|_| vm.new_overflow_error("BLOB longer than INT_MAX bytes"))?;
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

        vm.new_exception_msg_dict(typ, msg.into(), dict)
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
            .find(|&&x| x[6..].eq_ignore_ascii_case(s.as_bytes()))
            .map(|&x| x.as_ptr().cast())
            .ok_or_else(|| {
                vm.new_value_error(
                    "isolation_level string must be '', 'DEFERRED', 'IMMEDIATE', or 'EXCLUSIVE'",
                )
            })
    }

    fn lstrip_sql(sql: &[u8]) -> Option<&[u8]> {
        let mut pos = 0;

        // This loop is borrowed from the SQLite source code.
        while let Some(t_char) = sql.get(pos) {
            match t_char {
                b' ' | b'\t' | b'\x0c' | b'\n' | b'\r' => {
                    // Skip whitespace.
                    pos += 1;
                }

                // Skip line comments.
                b'-' if sql.get(pos + 1) == Some(&b'-') => {
                    pos += 2;
                    while let Some(&ch) = sql.get(pos) {
                        if ch == b'\n' {
                            break;
                        }
                        pos += 1;
                    }
                    let _ = sql.get(pos)?;
                }

                // Skip C style comments.
                b'/' if sql.get(pos + 1) == Some(&b'*') => {
                    pos += 2;
                    while let Some(&ch) = sql.get(pos) {
                        if ch == b'*' && sql.get(pos + 1) == Some(&b'/') {
                            break;
                        }
                        pos += 1;
                    }
                    let _ = sql.get(pos)?;
                    pos += 2;
                }
                _ => {
                    return Some(&sql[pos..]);
                }
            }
        }

        None
    }
}
