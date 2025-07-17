use rustpython_vm::RustPythonAllocator;

#[global_allocator]
static ALLOCATOR: RustPythonAllocator = RustPythonAllocator::new();

pub fn main() -> std::process::ExitCode {
    rustpython::run(|_vm| {})
}
