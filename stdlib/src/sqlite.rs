use crate::vm::{PyObjectRef, VirtualMachine};

// pub(crate) use _sqlite::make_module;
pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    // TODO: sqlite version check
    let module = _sqlite::make_module(vm);
    // module.set_item(needle, value, vm)
    // module.set_attr()
    for (name, code) in _sqlite::ERROR_CODES {
        let name = vm.ctx.new_str(*name);
        let code = vm.new_pyobj(*code);
        module.set_attr(name, code, vm).unwrap();
    }
    module
}

#[pymodule]
mod _sqlite {
    use rustpython_vm::{
        builtins::{PyStrRef, PyTypeRef},
        function::{ArgCallable, OptionalArg},
        object::PyObjectPayload,
        stdlib::os::PyPathLike,
        types::Constructor,
        PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,PyAtomicRef,PyObject
    };
    use sqlite3_sys::{
        sqlite3, sqlite3_complete, sqlite3_libversion, sqlite3_open_v2, sqlite3_threadsafe,
        SQLITE_OPEN_URI,
    };
    use std::{
        ffi::{CStr, CString},
        ptr::{addr_of_mut, null, null_mut},
    };

    #[pyattr]
    fn sqlite_version(_: &VirtualMachine) -> String {
        unsafe {
            let s = sqlite3_libversion();
            let s = CStr::from_ptr(s);
            s.to_str().unwrap().to_owned()
        }
    }

    #[pyattr]
    fn threadsafety(_: &VirtualMachine) -> i32 {
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
    const PARSE_DECLTYPES: i32 = 1;
    #[pyattr]
    const PARSE_COLNAMES: i32 = 2;

    #[pyattr]
    use sqlite3_sys::{
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
                use sqlite3_sys::$x;
            )*
            pub(crate) static ERROR_CODES: &[(&str, i32)] = &[
            $(
                (stringify!($x), sqlite3_sys::$x),
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
        SQLITE_OK_LOAD_PERMANENTLY
    );
    // SQLITE_IOERR_VNODE,
    // SQLITE_IOERR_AUTH,
    // SQLITE_IOERR_BEGIN_ATOMIC,
    // SQLITE_IOERR_COMMIT_ATOMIC,
    // SQLITE_IOERR_ROLLBACK_ATOMIC,
    // SQLITE_ERROR_MISSING_COLLSEQ,
    // SQLITE_ERROR_RETRY,
    // SQLITE_READONLY_CANTINIT,
    // SQLITE_READONLY_DIRECTORY,
    // SQLITE_CORRUPT_SEQUENCE,
    // SQLITE_LOCKED_VTAB,
    // SQLITE_CANTOPEN_DIRTYWAL,
    // SQLITE_ERROR_SNAPSHOT,
    // SQLITE_CANTOPEN_SYMLINK,
    // SQLITE_CONSTRAINT_PINNED,
    // SQLITE_OK_SYMLINK,
    // SQLITE_BUSY_TIMEOUT,
    // SQLITE_CORRUPT_INDEX,
    // SQLITE_IOERR_DATA,
    // SQLITE_IOERR_CORRUPTFS
    // TODO: update with sqlite-sys

    #[derive(FromArgs)]
    struct ConnectArgs {
        #[pyarg(any)]
        database: PyPathLike,
        #[pyarg(any, default = "5.0")]
        timeout: f64,
        #[pyarg(any, default = "0")]
        detect_types: i32,
        #[pyarg(any, optional)]
        isolation_level: Option<PyStrRef>,
        #[pyarg(any, default = "true")]
        check_same_thread: bool,
        #[pyarg(any, optional)]
        factory: Option<PyTypeRef>,
        #[pyarg(any, default = "0")]
        cached_statements: i32,
        #[pyarg(any, default = "false")]
        uri: bool,
    }

    #[pyfunction]
    fn connect(args: ConnectArgs, vm: &VirtualMachine) -> PyResult {
        // if let Some(factory) = args.factory.take() {}
        // Connection::py_new(Connection, args, vm)
        Connection::py_new(Connection::class(vm).to_owned(), args, vm)
    }

    #[pyfunction]
    fn complete_statement(statement: PyStrRef) -> bool {
        let s = CString::new(statement.as_str()).expect("CString::new from PyStrRef failed");
        let ret = unsafe { sqlite3_complete(s.as_ptr()) };
        ret == 1
    }

    #[pyfunction]
    fn enable_callback_tracebacks(flag: i32) {}

    #[pyfunction]
    fn register_adapter(typ: PyObjectRef, adapter: PyObjectRef) {}

    #[pyfunction]
    fn register_converter(typename: PyObjectRef, converter: PyObjectRef) {}

    #[pyattr]
    #[pyclass(name)]
    #[derive(PyPayload)]
    struct Connection {
        db: Sqlite,
        detect_types: i32,
        isolation_level: PyStrRef,
        check_same_thread: bool,
        thread_ident: u64,
        row_factory: PyObjectRef,
    }

    impl std::fmt::Debug for Connection {
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

    #[pyclass(with(Constructor))]
    impl Connection {
        fn new(args: ConnectArgs, vm: &VirtualMachine) -> PyResult<Self> {
            let path = args.database.into_cstring(vm)?;
            let db = Sqlite::open(path.as_ptr(), args.uri)?;
            let isolation_level = args
                .isolation_level
                .unwrap_or_else(|| vm.ctx.new_str("DEFERRED"));

            Ok(Self {
                db,
                detect_types: args.detect_types,
                isolation_level,
                check_same_thread: args.check_same_thread,
                thread_ident: crate::vm::stdlib::thread::get_ident(),
                row_factory: vm.ctx.none()
            })
        }

        #[pymethod]
        fn cursor(
            zelf: PyRef<Self>,
            factory: OptionalArg<ArgCallable>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Cursor>> {
            let cursor = if let OptionalArg::Present(factory) = factory {
                let cursor = factory.invoke((zelf.clone(),), vm)?;
                if let Some(cursor) = cursor.payload::<Cursor>() {
                    // if zelf
                }
            } else {
                Cursor::py_new(Cursor::class(vm).to_owned(), (zelf.clone(),), vm)?
            };
        }
    }

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug, PyPayload)]
    struct Cursor {
        connection: PyRef<Connection>,
        description: PyObjectRef,
        row_cast_map: Option<PyObjectRef>,
        arraysize: i32,
        lastrowid: PyObjectRef,
        rowcount: i64,
        row_factory: PyAtomicRef<PyObject>,
        // statement: PyRef<Statement>,
        closed: bool,
        // locked: bool,
        // initialized: bool,
    }

    #[pyclass(with(Constructor))]
    impl Cursor {
        fn new(connection: PyRef<Connection>, row_factory: PyObjectRef, vm: &VirtualMachine) -> Self{

            Self {
                connection,
                description: vm.ctx.none(),
                row_cast_map: None,
                arraysize: 1,
                lastrowid: vm.ctx.none(),
                rowcount: -1,
                row_factory,
                closed: false,
            }
        }
    }

    impl Constructor for Cursor {
        type Args = (PyRef<Connection>,);

        fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            Self::new(args.0, vm.ctx.none())
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug, PyPayload)]
    struct Row {}

    #[pyclass()]
    impl Row {}

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug, PyPayload)]
    struct Blob {}

    #[pyclass()]
    impl Blob {}

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug, PyPayload)]
    struct PrepareProtocol {}

    #[pyclass()]
    impl PrepareProtocol {}

    struct Sqlite {
        db: *mut sqlite3,
    }

    cfg_if::cfg_if! {
        if #[cfg(feature = "threading")] {
            unsafe impl Send for Sqlite {}
            unsafe impl Sync for Sqlite {}
        }
    }

    fn check_error(ret: i32) -> PyResult<()> {
        Ok(())
    }

    impl Sqlite {
        fn open(path: *const i8, uri: bool) -> PyResult<Self> {
            let mut db = null_mut();
            let ret = unsafe {
                sqlite3_open_v2(
                    path,
                    addr_of_mut!(db),
                    if uri { SQLITE_OPEN_URI } else { 0 },
                    null(),
                )
            };
            check_error(ret).map(|_| Self { db })
        }
    }
}
