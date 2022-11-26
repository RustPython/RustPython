use crate::vm::{PyObjectRef, VirtualMachine};

// pub(crate) use _sqlite::make_module;
pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    // TODO: sqlite version check
    let module = _sqlite::make_module(vm);

    for (name, code) in _sqlite::ERROR_CODES {
        let name = vm.ctx.new_str(*name);
        let code = vm.new_pyobj(*code);
        module.set_attr(name, code, vm).unwrap();
    }

    _sqlite::setup_module_exceptions(&module, vm);

    module
}

#[pymodule]
mod _sqlite {
    use rustpython_common::{
        atomic::{Ordering, PyAtomic, Radium},
        lock::{PyMappedMutexGuard, PyMutex, PyMutexGuard},
        static_cell,
    };
    use rustpython_vm::{
        builtins::{
            PyBaseException, PyBaseExceptionRef, PyDict, PyFloat, PyInt, PyStr, PyStrRef, PyType,
            PyTypeRef,
        },
        convert::IntoObject,
        function::{ArgCallable, ArgIterable, OptionalArg},
        object::PyObjectPayload,
        protocol::PySequence,
        stdlib::{os::PyPathLike, thread},
        types::Constructor,
        Py, PyAtomicRef, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        __exports::paste,
    };
    use sqlite3_sys::{
        sqlite3, sqlite3_bind_blob, sqlite3_bind_double, sqlite3_bind_int64, sqlite3_bind_null,
        sqlite3_bind_parameter_count, sqlite3_bind_parameter_name, sqlite3_bind_text,
        sqlite3_changes, sqlite3_close_v2, sqlite3_complete, sqlite3_data_count, sqlite3_db_handle,
        sqlite3_errcode, sqlite3_errmsg, sqlite3_extended_errcode, sqlite3_finalize,
        sqlite3_get_autocommit, sqlite3_last_insert_rowid, sqlite3_libversion, sqlite3_limit,
        sqlite3_open_v2, sqlite3_prepare_v2, sqlite3_reset, sqlite3_step, sqlite3_stmt,
        sqlite3_threadsafe, SQLITE_OPEN_CREATE, SQLITE_OPEN_READWRITE, SQLITE_OPEN_URI,
        SQLITE_TRANSIENT,
    };
    use std::{
        ffi::{c_int, c_longlong, CStr, CString},
        fmt::Debug,
        ops::Deref,
        ptr::{addr_of_mut, null, null_mut, NonNull},
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
                    fn [<new_ $x:snake>](vm: &VirtualMachine, msg: String) -> PyBaseExceptionRef {
                        vm.new_exception_msg([<$x:snake _type>]().to_owned(), msg)
                    }
                    fn [<$x:snake _type>]() -> &'static Py<PyType> {
                        [<$x:snake:upper>].get().expect("exception type not initialize")
                    }
                )*
                pub fn setup_module_exceptions(module: &PyObject, vm: &VirtualMachine) {
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
    fn complete_statement(statement: PyStrRef, vm: &VirtualMachine) -> PyResult<bool> {
        let s = to_cstring(&statement, vm)?;
        let ret = unsafe { sqlite3_complete(s.as_ptr()) };
        Ok(ret == 1)
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
        db: PyMutex<Option<Sqlite>>,
        detect_types: i32,
        isolation_level: PyStrRef,
        check_same_thread: bool,
        thread_ident: u64,
        row_factory: PyObjectRef,
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

    #[pyclass(with(Constructor))]
    impl Connection {
        fn new(args: ConnectArgs, vm: &VirtualMachine) -> PyResult<Self> {
            let path = args.database.into_cstring(vm)?;
            let db = SqliteRaw::open(path.as_ptr(), args.uri, vm)?.into();
            let isolation_level = args
                .isolation_level
                .unwrap_or_else(|| vm.ctx.new_str("DEFERRED"));

            Ok(Self {
                db: PyMutex::new(Some(db)),
                detect_types: args.detect_types,
                isolation_level,
                check_same_thread: args.check_same_thread,
                thread_ident: thread::get_ident(),
                row_factory: vm.ctx.none(),
            })
        }

        fn db_lock(&self, vm: &VirtualMachine) -> PyResult<PyMappedMutexGuard<Sqlite>> {
            let guard = self.db.lock();
            if guard.is_some() {
                Ok(PyMutexGuard::map(guard, |x| unsafe {
                    x.as_mut().unwrap_unchecked()
                }))
            } else {
                Err(new_programming_error(vm, "".to_owned()))
            }
        }

        #[pymethod]
        fn cursor(
            zelf: PyRef<Self>,
            factory: OptionalArg<ArgCallable>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Cursor>> {
            let cursor = if let OptionalArg::Present(factory) = factory {
                let cursor = factory.invoke((zelf.clone(),), vm)?;
                let cursor = cursor.downcast::<Cursor>().map_err(|x| {
                    vm.new_type_error(format!("factory must return a cursor, not {}", x.class()))
                })?;
                if !vm.is_none(&zelf.row_factory) {
                    cursor
                        .row_factory
                        .swap_to_temporary_refs(zelf.row_factory.clone(), vm);
                }
                cursor
            } else {
                Cursor::new(zelf.clone(), zelf.row_factory.clone(), vm).into_ref(vm)
            };
            Ok(cursor)
        }

        #[pymethod]
        fn close(&self) {
            self.db.lock().take();
        }

        fn begin_transaction(&self, vm: &VirtualMachine) -> PyResult<()> {
            let mut s = b"BEGIN ".to_vec();
            s.extend_from_slice(self.isolation_level.as_str().as_bytes());
            let db = self.db_lock(vm)?;
            let statement = db.prepare(s.as_ptr().cast(), null_mut(), vm)?;
            let statement = SqliteStatementRaw::from(statement);
            let rc = statement.step();
            db.check(rc, vm)
        }

        fn check_thread(&self, vm: &VirtualMachine) -> PyResult<()> {
            if self.check_same_thread && (thread::get_ident() != self.thread_ident) {
                Err(new_programming_error(
                    vm,
                    "SQLite objects created in a thread can only be used in that same thread."
                        .to_owned(),
                ))
            } else {
                Ok(())
            }
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
        lastrowid: PyAtomic<i64>,
        rowcount: PyAtomic<i64>,
        row_factory: PyAtomicRef<PyObject>,
        statement: PyAtomicRef<Option<Statement>>,
        closed: bool,
        // locked: bool,
    }

    #[pyclass(with(Constructor))]
    impl Cursor {
        fn new(
            connection: PyRef<Connection>,
            row_factory: PyObjectRef,
            vm: &VirtualMachine,
        ) -> Self {
            Self {
                connection,
                description: vm.ctx.none(),
                row_cast_map: None,
                arraysize: 1,
                lastrowid: Radium::new(-1),
                rowcount: Radium::new(-1),
                row_factory: PyAtomicRef::from(row_factory),
                statement: PyAtomicRef::from(None),
                closed: false,
            }
        }

        #[pymethod]
        fn execute(
            &self,
            sql: PyStrRef,
            parameters: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // let parameters: Vec<PyObjectRef> = parameters
            //     .iter(vm)?
            //     .collect::<PyResult<Vec<PyObjectRef>>>()?;
            // let stmt = &mut *self.statement.lock();
            if let Some(stmt) = self.statement.deref() {
                stmt.lock().reset();
            }

            let stmt = Statement::new(&self.connection, &sql, vm)?.into_ref(vm);
            self.statement
                .swap_to_temporary_refs(Some(stmt.clone()), vm);

            self.rowcount
                .store(if stmt.is_dml { 0 } else { -1 }, Ordering::Relaxed);

            let db = self.connection.db_lock(vm)?;

            if !self.connection.isolation_level.is_empty() && stmt.is_dml && db.is_autocommit() {
                self.connection.begin_transaction(vm)?;
            }

            let st = stmt.lock();
            st.bind_parameters(&parameters, vm)?;
            let ret = st.step();
            if ret != SQLITE_DONE && ret != SQLITE_ROW {
                return Err(db.error_extended(vm));
            }

            // TODO: descreption

            if ret == SQLITE_DONE {
                if stmt.is_dml {
                    self.rowcount
                        .fetch_add(db.changes() as i64, Ordering::Relaxed);
                }
                st.reset();
            }

            let lastrowid = db.lastrowid();
            self.lastrowid.store(lastrowid, Ordering::Relaxed);

            Ok(())
        }

        fn fetch_one_row(&self) -> PyResult<()> {
            Ok(())
            // let num_cols = self.
        }

        fn not_close(&self) -> PyResult<()> {
            Ok(())
        }

        #[pygetset]
        fn connection(&self) -> PyRef<Connection> {
            self.connection.clone()
        }

        #[pygetset]
        fn lastrowid(&self) -> i64 {
            self.lastrowid.load(Ordering::Relaxed)
        }

        #[pygetset]
        fn rowcount(&self) -> i64 {
            self.rowcount.load(Ordering::Relaxed)
        }
    }

    impl Constructor for Cursor {
        type Args = (PyRef<Connection>,);

        fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            Self::new(args.0, vm.ctx.none(), vm)
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
        fn new(connection: &Connection, sql: &PyStr, vm: &VirtualMachine) -> PyResult<Self> {
            let sql_cstr = to_cstring(sql, vm)?;
            let sql_len = (sql.byte_len() + 1) as i32;

            let db = connection.db_lock(vm)?;

            let max_len = db.limit(SQLITE_LIMIT_SQL_LENGTH);
            if sql_len > max_len {
                return Err(new_data_error(vm, "query string is too large".to_owned()));
            }

            let mut tail = null();
            let st = db.prepare(sql_cstr.as_ptr(), addr_of_mut!(tail), vm)?;
            let st: SqliteStatement = SqliteStatementRaw::from(st).into();

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

            Ok(Self {
                st: PyMutex::from(st),
                is_dml,
            })
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

        fn open(path: *const i8, uri: bool, vm: &VirtualMachine) -> PyResult<Self> {
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

        fn prepare(
            self,
            sql: *const i8,
            tail: *mut *const i8,
            vm: &VirtualMachine,
        ) -> PyResult<*mut sqlite3_stmt> {
            let mut st = null_mut();
            let ret = unsafe { sqlite3_prepare_v2(self.db, sql, -1, addr_of_mut!(st), tail) };
            self.check(ret, vm).map(|_| st)
        }

        fn limit(self, id: i32) -> i32 {
            unsafe { sqlite3_limit(self.db, id, -1) }
        }

        fn is_autocommit(self) -> bool {
            unsafe { sqlite3_get_autocommit(self.db) != 0 }
        }

        fn changes(self) -> c_int {
            unsafe { sqlite3_changes(self.db) }
        }

        fn lastrowid(self) -> c_longlong {
            unsafe { sqlite3_last_insert_rowid(self.db) }
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
            let ret = if vm.is_none(parameter) {
                unsafe { sqlite3_bind_null(self.st, pos) }
            } else if let Some(val) = parameter.payload::<PyInt>() {
                let val = val.try_to_primitive::<i64>(vm)?;
                unsafe { sqlite3_bind_int64(self.st, pos, val) }
            } else if let Some(val) = parameter.payload::<PyFloat>() {
                let val = val.to_f64();
                unsafe { sqlite3_bind_double(self.st, pos, val) }
            } else if let Some(val) = parameter.payload::<PyStr>() {
                let s = to_cstring(val, vm)?;
                unsafe { sqlite3_bind_text(self.st, pos, s.as_ptr(), -1, None) }
            } else {
                return Err(new_programming_error(
                    vm,
                    format!(
                        "Error binding parameter {}: type '{}' is not supported",
                        pos,
                        parameter.class()
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
                    "parameters are unsupported type".to_owned(),
                ))
            }
        }

        fn bind_parameters_name(self, dict: &Py<PyDict>, vm: &VirtualMachine) -> PyResult<()> {
            let num_needed = unsafe { sqlite3_bind_parameter_count(self.st) };

            for i in 1..=num_needed {
                let name = unsafe { sqlite3_bind_parameter_name(self.st, i) };
                let name = unsafe { name.add(1) };
                let name = unsafe { CStr::from_ptr(name) };
                let name = name.to_str().unwrap();

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
                let val = seq.get_item(i as isize, vm)?;
                self.bind_parameter(i, &val, vm)?;
            }
            Ok(())
        }
    }

    fn to_cstring(s: &PyStr, vm: &VirtualMachine) -> PyResult<CString> {
        CString::new(s.as_str())
            .map_err(|_| vm.new_value_error("embedded null character".to_owned()))
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
