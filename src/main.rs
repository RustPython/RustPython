use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

pub fn main() -> std::process::ExitCode {
    rustpython::run(|_vm| {})
}
