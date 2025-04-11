pub(crate) use _datetime::make_module;

#[pymodule]
mod _datetime {
    use rustpython_vm::VirtualMachine;

    #[pyattr]
    pub const MINYEAR: i32 = 1;
    #[pyattr]
    pub const MAXYEAR: i32 = 9999;
}
