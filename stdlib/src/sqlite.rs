pub(crate) use _sqlite::make_module;
// pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
//     // TODO: sqlite version check
//     _sqlite::make_module(vm)
// }

#[pymodule]
mod _sqlite {
    use crate::vm::VirtualMachine;
    use std::ffi::CStr;

    #[pyattr]
    fn sqlite_version(_: &VirtualMachine) -> String {
        unsafe {
            let s = sqlite3_sys::sqlite3_libversion();
            let s = CStr::from_ptr(s);
            s.to_str().unwrap().to_owned()
        }
    }

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
}
