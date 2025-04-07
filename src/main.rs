#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
mod _alloc {
    use mimalloc::MiMalloc;

    #[global_allocator]
    static GLOBAL: MiMalloc = MiMalloc;
}

pub fn main() -> std::process::ExitCode {
    rustpython::run(|_vm| {})
}
